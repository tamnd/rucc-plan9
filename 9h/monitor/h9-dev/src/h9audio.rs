//! The audio output of the 9h kernel (`audioh9.c`, plan 11.6). The kernel gives samples to the
//! monitor with `H9audiowrite`, and asks how many bytes wait with `H9audiobuffered`. The samples
//! are signed 16-bit little-endian stereo, as `/dev/audio` of 9front, at the speed of the call.
//!
//! The samples wait in a queue of the monitor. A player takes them from the queue on its own
//! thread and plays them. The kernel waits while the queue has more than `delay` samples, so the
//! player sets the pace. This module has two players that need no audio device: [`Clock`] takes
//! the samples at the speed of the stream, and writes them to a WAV file or drops them, and
//! [`Program`] writes them to the standard input of a host program, such as `pacat` or `aplay`.
//! The player of `CoreAudio` is in the `9h` command, because it calls the framework.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use h9_vm::Ram;

use crate::Device;
use crate::h9;

/// The bytes of one sample: two channels of 16 bits.
pub const FRAME: usize = 4;

/// The samples that wait to play.
#[derive(Debug, Default)]
pub struct Queue {
    state: Mutex<State>,
    ready: Condvar,
}

#[derive(Debug, Default)]
struct State {
    pcm: VecDeque<u8>,
    speed: u32,
}

impl Queue {
    /// The bytes that wait. It is always a number of whole samples.
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.lock().unwrap().pcm.len()
    }

    /// True when no byte waits.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The speed of the stream in samples a second, 0 before the first write.
    #[must_use]
    pub fn speed(&self) -> u32 {
        self.state.lock().unwrap().speed
    }

    fn push(&self, pcm: &[u8], speed: u32) {
        let mut s = self.state.lock().unwrap();
        s.pcm.extend(pcm);
        s.speed = speed;
        self.ready.notify_all();
    }

    /// Move the first whole samples of the queue to `out`, at most `out.len()` bytes, and return
    /// the number of bytes.
    pub fn take(&self, out: &mut [u8]) -> usize {
        let mut s = self.state.lock().unwrap();
        Queue::take_locked(&mut s, out)
    }

    fn take_locked(s: &mut State, out: &mut [u8]) -> usize {
        let n = s.pcm.len().min(out.len()) / FRAME * FRAME;
        for (o, b) in out.iter_mut().zip(s.pcm.drain(..n)) {
            *o = b;
        }
        n
    }

    /// As [`Queue::take`], but wait at most `wait` for a sample when the queue is empty.
    pub fn take_wait(&self, out: &mut [u8], wait: Duration) -> usize {
        let s = self.state.lock().unwrap();
        let (mut s, _) = self
            .ready
            .wait_timeout_while(s, wait, |s| s.pcm.is_empty())
            .unwrap();
        Queue::take_locked(&mut s, out)
    }
}

/// A player of the samples of a [`Queue`].
pub trait Player: Send {
    /// The stream now has `speed` samples a second. The device calls it before the first
    /// sample, and each time the speed changes.
    fn speed(&mut self, speed: u32);
}

/// A thread of a player, which stops when the player is dropped.
#[derive(Debug)]
struct Worker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    fn spawn(f: impl FnOnce(&AtomicBool) + Send + 'static) -> Worker {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let thread = std::thread::Builder::new()
            .name("9h-audio".into())
            .spawn(move || f(&s))
            .ok();
        Worker { stop, thread }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A WAV file of signed 16-bit stereo samples. The header has the speed of the first samples,
/// and each write puts the right sizes in the header.
#[derive(Debug)]
pub struct Wav {
    file: File,
    bytes: u32,
}

impl Wav {
    /// Make the file `path`, with no samples.
    pub fn create(path: &str) -> std::io::Result<Wav> {
        Ok(Wav {
            file: File::create(path)?,
            bytes: 0,
        })
    }

    fn header(speed: u32, bytes: u32) -> [u8; 44] {
        let mut h = [0u8; 44];
        let mut put = |at: usize, b: &[u8]| h[at..at + b.len()].copy_from_slice(b);
        put(0, b"RIFF");
        put(4, &(36 + bytes).to_le_bytes());
        put(8, b"WAVEfmt ");
        put(16, &16u32.to_le_bytes());
        put(20, &1u16.to_le_bytes());
        put(22, &2u16.to_le_bytes());
        put(24, &speed.to_le_bytes());
        put(28, &(speed * FRAME as u32).to_le_bytes());
        put(32, &(FRAME as u16).to_le_bytes());
        put(34, &16u16.to_le_bytes());
        put(36, b"data");
        put(40, &bytes.to_le_bytes());
        h
    }

    /// Add the samples `pcm`, at `speed` samples a second when they are the first.
    pub fn write(&mut self, pcm: &[u8], speed: u32) -> std::io::Result<()> {
        if self.bytes == 0 {
            self.file.write_all(&Wav::header(speed, 0))?;
        }
        self.file.write_all(pcm)?;
        self.bytes = self.bytes.saturating_add(pcm.len() as u32);
        self.file.seek(SeekFrom::Start(4))?;
        self.file.write_all(&(36 + self.bytes).to_le_bytes())?;
        self.file.seek(SeekFrom::Start(40))?;
        self.file.write_all(&self.bytes.to_le_bytes())?;
        self.file.seek(SeekFrom::End(0))?;
        Ok(())
    }
}

/// The player with no audio device. Its thread takes the samples at the speed of the stream, as
/// a sound card does, and writes them to a WAV file, or drops them when it has no file. When the
/// queue is empty, the clock waits for the next sample, so a pause of the guest does not make
/// the next samples go faster.
#[derive(Debug)]
pub struct Clock {
    _worker: Worker,
}

impl Clock {
    /// The tick of the thread.
    pub const TICK: Duration = Duration::from_millis(10);

    /// A clock on `queue` that writes the samples to `wav` when it has one.
    #[must_use]
    pub fn new(queue: Arc<Queue>, mut wav: Option<Wav>) -> Clock {
        let worker = Worker::spawn(move |stop| {
            let mut buf = vec![0u8; 1 << 16];
            let mut last = Instant::now();
            // The samples that the clock can take, in units of one second / 1e9.
            let mut credit: u128 = 0;
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Clock::TICK);
                let now = Instant::now();
                let speed = u128::from(queue.speed());
                credit += now.duration_since(last).as_nanos() * speed;
                last = now;
                let frames = (credit / 1_000_000_000) as usize;
                let n = queue.take(&mut buf[..(frames * FRAME).min(1 << 16)]);
                if n == 0 {
                    credit = 0;
                    continue;
                }
                credit -= (n / FRAME) as u128 * 1_000_000_000;
                if let Some(w) = &mut wav
                    && let Err(e) = w.write(&buf[..n], queue.speed())
                {
                    eprintln!("9h: the audio file: {e}");
                    wav = None;
                }
            }
        });
        Clock { _worker: worker }
    }
}

impl Player for Clock {
    fn speed(&mut self, _: u32) {}
}

/// The player that writes the samples to the standard input of a host program. The program
/// sets the pace, because a pipe that is full stops the write.
#[derive(Debug)]
pub struct Program {
    queue: Arc<Queue>,
    argv: Vec<String>,
    run: Option<(Child, Worker)>,
}

impl Program {
    /// The programs that play raw samples from the standard input, with `{rate}` for the speed,
    /// in the order of preference.
    pub const PROGRAMS: &[&str] = &[
        "pacat --raw --format=s16le --channels=2 --latency-msec=50 --rate={rate}",
        "pw-cat --playback --format=s16 --channels=2 --rate={rate} -",
        "aplay -q -t raw -f S16_LE -c 2 -r {rate}",
    ];

    /// The first program of [`Program::PROGRAMS`] that is on the path, as words.
    #[must_use]
    pub fn find() -> Option<Vec<String>> {
        let path = std::env::var_os("PATH")?;
        Program::PROGRAMS.iter().find_map(|p| {
            let argv: Vec<String> = p.split(' ').map(String::from).collect();
            std::env::split_paths(&path)
                .any(|d| d.join(&argv[0]).is_file())
                .then_some(argv)
        })
    }

    /// A player on `queue` that runs `argv`, with `{rate}` in a word for the speed.
    #[must_use]
    pub fn new(queue: Arc<Queue>, argv: Vec<String>) -> Program {
        Program {
            queue,
            argv,
            run: None,
        }
    }

    fn feed(queue: &Queue, mut w: ChildStdin, stop: &AtomicBool) {
        let mut buf = vec![0u8; 4096];
        while !stop.load(Ordering::Relaxed) {
            let n = queue.take_wait(&mut buf, Duration::from_millis(100));
            if n > 0 && w.write_all(&buf[..n]).is_err() {
                eprintln!("9h: the audio program stopped");
                return;
            }
        }
    }
}

impl Player for Program {
    fn speed(&mut self, speed: u32) {
        if let Some((mut child, worker)) = self.run.take() {
            let _ = child.kill();
            drop(worker);
            let _ = child.wait();
        }
        let argv: Vec<String> = self
            .argv
            .iter()
            .map(|a| a.replace("{rate}", &speed.to_string()))
            .collect();
        let child = Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn();
        match child {
            Ok(mut child) => {
                let w = child.stdin.take().expect("piped");
                let q = self.queue.clone();
                let worker = Worker::spawn(move |stop| Program::feed(&q, w, stop));
                self.run = Some((child, worker));
            }
            Err(e) => eprintln!("9h: {}: {e}", argv[0]),
        }
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        if let Some((mut child, worker)) = self.run.take() {
            let _ = child.kill();
            drop(worker);
            let _ = child.wait();
        }
    }
}

/// The audio device.
pub struct H9Audio {
    queue: Arc<Queue>,
    player: Box<dyn Player>,
    speed: u32,
    /// The bytes of a sample that the last write did not complete.
    carry: Vec<u8>,
}

impl H9Audio {
    /// A device with the player that `make` gives for the queue of the device.
    pub fn new(make: impl FnOnce(Arc<Queue>) -> Box<dyn Player>) -> H9Audio {
        let queue = Arc::new(Queue::default());
        H9Audio {
            player: make(queue.clone()),
            queue,
            speed: 0,
            carry: Vec::new(),
        }
    }

    /// Scale each sample of `pcm` by the volume, left in the low byte of `vol` and right in the
    /// next byte, each 0 to 100.
    fn scale(pcm: &mut [u8], vol: u64) {
        let left = i32::from((vol & 0xff).min(100) as u8);
        let right = i32::from((vol >> 8 & 0xff).min(100) as u8);
        if left == 100 && right == 100 {
            return;
        }
        for f in pcm.as_chunks_mut::<FRAME>().0 {
            for (c, v) in [(0, left), (2, right)] {
                let s = i32::from(i16::from_le_bytes([f[c], f[c + 1]])) * v / 100;
                f[c..c + 2].copy_from_slice(&(s as i16).to_le_bytes());
            }
        }
    }

    fn write(&mut self, pcm: &[u8], speed: u64, vol: u64) -> u64 {
        let speed = speed.clamp(1, 1_000_000) as u32;
        if speed != self.speed {
            self.speed = speed;
            self.player.speed(speed);
        }
        let room = h9::AUDIO_QUEUE as usize - self.queue.len() - self.carry.len();
        let pcm = &pcm[..pcm.len().min(room)];
        let mut all = std::mem::take(&mut self.carry);
        all.extend_from_slice(pcm);
        let whole = all.len() / FRAME * FRAME;
        self.carry = all.split_off(whole);
        H9Audio::scale(&mut all, vol);
        self.queue.push(&all, speed);
        pcm.len() as u64
    }
}

impl Device for H9Audio {
    fn name(&self) -> &'static str {
        "h9audio"
    }

    fn read(&mut self, _: u64, _: u8) -> u64 {
        u64::MAX
    }

    fn write(&mut self, _: u64, _: u8, _: u64) {}

    fn call(&mut self, x: &[u64; 6], ram: &Ram) -> Option<u64> {
        Some(match x[0] as u32 {
            h9::AUDIO_WRITE => {
                if x[2] > h9::AUDIO_MAX {
                    return Some(h9::IO_ERROR);
                }
                let mut b = vec![0; x[2] as usize];
                if !b.is_empty() && !ram.read(x[1], &mut b) {
                    return Some(h9::BAD_ADDRESS);
                }
                self.write(&b, x[3], x[4])
            }
            h9::AUDIO_BUFFERED => (self.queue.len() + self.carry.len()) as u64,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, H9Audio, Player, Queue, Wav};
    use crate::{Bus, Device, h9};
    use h9_vm::Ram;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// A player that keeps the speeds it was given.
    struct Speeds(Arc<Mutex<Vec<u32>>>);
    impl Player for Speeds {
        fn speed(&mut self, s: u32) {
            self.0.lock().unwrap().push(s);
        }
    }

    fn pcm(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn a_write_queues_whole_samples_and_scales_them() {
        let ram = Ram::new(0x4000_0000, 0x1_0000).unwrap();
        let speeds = Arc::new(Mutex::new(Vec::new()));
        let mut queue = None;
        let s = speeds.clone();
        let mut dev = H9Audio::new(|q| {
            queue = Some(q);
            Box::new(Speeds(s))
        });
        let queue = queue.unwrap();
        let full = 100 + 100 * 256;
        assert!(ram.write(0x4000_0000, &pcm(12)));
        let mut call = |f: u32, a: u64, n: u64, sp: u64, v: u64| {
            dev.call(&[u64::from(f), a, n, sp, v, 0], &ram).unwrap()
        };
        assert_eq!(call(h9::AUDIO_WRITE, 0x4000_0000, 10, 44100, full), 10);
        assert_eq!(queue.len(), 8, "two bytes wait for the rest of a sample");
        assert_eq!(call(h9::AUDIO_BUFFERED, 0, 0, 0, 0), 10);
        assert_eq!(call(h9::AUDIO_WRITE, 0x4000_000a, 2, 44100, full), 2);
        assert_eq!(queue.len(), 12);
        let mut out = [0u8; 12];
        assert_eq!(queue.take(&mut out), 12);
        assert_eq!(out.to_vec(), pcm(12));

        // A sample of 1000 and -1000 at 50 and 25.
        let mut f = Vec::new();
        f.extend_from_slice(&1000i16.to_le_bytes());
        f.extend_from_slice(&(-1000i16).to_le_bytes());
        assert!(ram.write(0x4000_0100, &f));
        assert_eq!(
            call(h9::AUDIO_WRITE, 0x4000_0100, 4, 22050, 50 + 25 * 256),
            4
        );
        let mut out = [0u8; 4];
        assert_eq!(queue.take(&mut out), 4);
        assert_eq!(i16::from_le_bytes([out[0], out[1]]), 500);
        assert_eq!(i16::from_le_bytes([out[2], out[3]]), -250);
        assert_eq!(*speeds.lock().unwrap(), [44100, 22050]);

        assert_eq!(
            call(h9::AUDIO_WRITE, 0, h9::AUDIO_MAX + 1, 44100, full),
            h9::IO_ERROR
        );
        assert_eq!(
            call(h9::AUDIO_WRITE, 0x5000_0000, 4, 44100, full),
            h9::BAD_ADDRESS
        );
    }

    #[test]
    fn the_queue_takes_no_more_than_its_size() {
        let ram = Ram::new(0x4000_0000, 0x1_0000).unwrap();
        let mut bus = Bus::new();
        bus.add_hyper(Box::new(H9Audio::new(|_| Box::new(Speeds(Arc::default())))));
        let mut took = 0;
        loop {
            let n = bus.call(
                &[
                    u64::from(h9::AUDIO_WRITE),
                    0x4000_0000,
                    0x1_0000,
                    44100,
                    0xffff,
                    0,
                ],
                &ram,
            );
            took += n;
            if n < 0x1_0000 {
                break;
            }
        }
        assert_eq!(took, h9::AUDIO_QUEUE);
        assert_eq!(
            bus.call(
                &[u64::from(h9::AUDIO_WRITE), 0x4000_0000, 4, 44100, 0, 0],
                &ram
            ),
            0
        );
    }

    #[test]
    fn the_clock_plays_at_the_speed_into_a_wav_file() {
        let dir = std::env::temp_dir().join(format!("h9audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.wav");
        let ram = Ram::new(0x4000_0000, 0x1_0000).unwrap();
        // 0.1 s at 8000 samples a second.
        let samples = pcm(3200);
        assert!(ram.write(0x4000_0000, &samples));
        let wav = Wav::create(path.to_str().unwrap()).unwrap();
        let mut queue: Option<Arc<Queue>> = None;
        let mut dev = H9Audio::new(|q| {
            queue = Some(q.clone());
            Box::new(Clock::new(q, Some(wav)))
        });
        let queue = queue.unwrap();
        let start = Instant::now();
        let r = dev.call(
            &[
                u64::from(h9::AUDIO_WRITE),
                0x4000_0000,
                3200,
                8000,
                0xffff,
                0,
            ],
            &ram,
        );
        assert_eq!(r, Some(3200));
        while !queue.is_empty() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "the clock is stuck"
            );
            std::thread::sleep(Clock::TICK);
        }
        assert!(
            start.elapsed() >= Duration::from_millis(80),
            "the clock played 0.1 s in {:?}",
            start.elapsed()
        );
        drop(dev);
        let b = std::fs::read(&path).unwrap();
        assert_eq!(&b[..4], b"RIFF");
        assert_eq!(&b[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 8000);
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 3200);
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 3236);
        assert_eq!(&b[44..], &samples[..]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
