//! The player of `#A/audio` on macOS (plan 11.6). It is an output queue of `AudioToolbox` on the
//! default output device. The framework calls [`fill`] on a thread of its own each time a buffer
//! has played, and `fill` puts the next samples of the queue of the device in the buffer, or
//! silence when the queue is empty.

use std::ffi::c_void;
use std::sync::Arc;

use h9_dev::h9audio::{FRAME, Player, Queue};

#[repr(C)]
struct StreamFormat {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[repr(C)]
struct QueueBuffer {
    capacity: u32,
    data: *mut c_void,
    size: u32,
    user_data: *mut c_void,
    packet_capacity: u32,
    packets: *mut c_void,
    packet_count: u32,
}

type QueueRef = *mut c_void;
type Callback = extern "C" fn(*mut c_void, QueueRef, *mut QueueBuffer);

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioQueueNewOutput(
        format: *const StreamFormat,
        callback: Callback,
        user_data: *mut c_void,
        run_loop: *const c_void,
        run_loop_mode: *const c_void,
        flags: u32,
        queue: *mut QueueRef,
    ) -> i32;
    fn AudioQueueAllocateBuffer(queue: QueueRef, size: u32, buffer: *mut *mut QueueBuffer) -> i32;
    fn AudioQueueEnqueueBuffer(
        queue: QueueRef,
        buffer: *mut QueueBuffer,
        packets: u32,
        descriptions: *const c_void,
    ) -> i32;
    fn AudioQueueStart(queue: QueueRef, start: *const c_void) -> i32;
    fn AudioQueueDispose(queue: QueueRef, immediate: u8) -> i32;
}

/// `kAudioFormatLinearPCM`.
const LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");
/// `kLinearPCMFormatFlagIsSignedInteger | kLinearPCMFormatFlagIsPacked`.
const SIGNED_PACKED: u32 = 0x4 | 0x8;
/// The samples of one buffer: 1024, about 23 ms at 44100 Hz.
const BUFFER_FRAMES: u32 = 1024;
/// The buffers of the output queue.
const BUFFERS: usize = 4;

extern "C" fn fill(user_data: *mut c_void, queue: QueueRef, buffer: *mut QueueBuffer) {
    // SAFETY: user_data is the pointer of `Arc::into_raw` in `CoreAudio::speed`, and the
    // reference that it holds lives until `AudioQueueDispose` returns, after the last call.
    let q = unsafe { &*user_data.cast::<Queue>() };
    // SAFETY: the framework gives a buffer of this queue, which no other code uses during the
    // call.
    let b = unsafe { &mut *buffer };
    let cap = b.capacity as usize;
    // SAFETY: data has capacity bytes, as `AudioQueueAllocateBuffer` made it.
    let out = unsafe { std::slice::from_raw_parts_mut(b.data.cast::<u8>(), cap) };
    let n = q.take(out);
    out[n..].fill(0);
    b.size = cap as u32;
    // SAFETY: the buffer is of this queue. A call after the dispose fails, and is not made,
    // because the dispose stops the calls first.
    unsafe { AudioQueueEnqueueBuffer(queue, buffer, 0, std::ptr::null()) };
}

/// The output queue of `AudioToolbox` of a stream.
struct Output {
    queue: QueueRef,
    user_data: *const Queue,
}

// SAFETY: the queue reference is only used by the device thread, to dispose of it.
unsafe impl Send for Output {}

impl Drop for Output {
    fn drop(&mut self) {
        // SAFETY: the queue is live, and the immediate dispose returns after the last call of
        // `fill`, so the reference of user_data is no longer used.
        unsafe {
            AudioQueueDispose(self.queue, 1);
            drop(Arc::from_raw(self.user_data));
        }
    }
}

/// The player on the default output device of macOS.
pub struct CoreAudio {
    queue: Arc<Queue>,
    out: Option<Output>,
}

impl CoreAudio {
    /// A player on `queue`. The output starts at the first sample.
    #[must_use]
    pub fn new(queue: Arc<Queue>) -> CoreAudio {
        CoreAudio { queue, out: None }
    }

    fn start(&self, speed: u32) -> Result<Output, i32> {
        let format = StreamFormat {
            sample_rate: f64::from(speed),
            format_id: LINEAR_PCM,
            format_flags: SIGNED_PACKED,
            bytes_per_packet: FRAME as u32,
            frames_per_packet: 1,
            bytes_per_frame: FRAME as u32,
            channels_per_frame: 2,
            bits_per_channel: 16,
            reserved: 0,
        };
        let user_data = Arc::into_raw(self.queue.clone());
        let mut queue: QueueRef = std::ptr::null_mut();
        // SAFETY: the format and the out pointer are valid for the call, and a null run loop
        // makes the framework call `fill` on its own thread.
        let r = unsafe {
            AudioQueueNewOutput(
                &raw const format,
                fill,
                user_data.cast_mut().cast(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &raw mut queue,
            )
        };
        if r != 0 {
            // SAFETY: the framework did not keep the pointer, because the call failed.
            drop(unsafe { Arc::from_raw(user_data) });
            return Err(r);
        }
        let out = Output { queue, user_data };
        for _ in 0..BUFFERS {
            let mut b: *mut QueueBuffer = std::ptr::null_mut();
            // SAFETY: the queue is live and b is a valid out pointer.
            let r = unsafe {
                AudioQueueAllocateBuffer(queue, BUFFER_FRAMES * FRAME as u32, &raw mut b)
            };
            if r != 0 {
                return Err(r);
            }
            fill(user_data.cast_mut().cast(), queue, b);
        }
        // SAFETY: the queue is live, and a null time starts it now.
        let r = unsafe { AudioQueueStart(queue, std::ptr::null()) };
        if r != 0 {
            return Err(r);
        }
        Ok(out)
    }
}

impl Player for CoreAudio {
    fn speed(&mut self, speed: u32) {
        self.out = None;
        match self.start(speed) {
            Ok(o) => self.out = Some(o),
            Err(e) => eprintln!("9h: the audio output of macOS failed with the error {e}"),
        }
    }
}
