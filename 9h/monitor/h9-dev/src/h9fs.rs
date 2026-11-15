//! The host's files of the 9h kernel (`#Z/hostfs` of `devh9.c`, plan 10.5). The kernel writes
//! 9P2000 messages with `H9fswrite` and reads the replies with `H9fsread`. The server answers
//! each whole message in the call that gives its last byte, so a reply always waits by the time
//! the kernel reads for it. The kernel thus needs no interrupt for the channel.

use std::collections::VecDeque;

use h9_9p::{MAXMSIZE, Options, Server};
use h9_vm::Ram;

use crate::Device;
use crate::h9;

/// The device of the host's files.
pub struct H9Fs {
    opts: Options,
    server: Server,
    input: Vec<u8>,
    output: VecDeque<u8>,
}

impl H9Fs {
    /// A device that serves the host directory of `opts`.
    #[must_use]
    pub fn new(opts: Options) -> H9Fs {
        H9Fs {
            server: Server::new(opts.clone()),
            opts,
            input: Vec::new(),
            output: VecDeque::new(),
        }
    }

    fn take(&mut self, b: &[u8]) -> Result<(), ()> {
        self.input.extend_from_slice(b);
        while self.input.len() >= 4 {
            let n =
                u32::from_le_bytes([self.input[0], self.input[1], self.input[2], self.input[3]]);
            if !(7..=MAXMSIZE).contains(&n) {
                self.input.clear();
                return Err(());
            }
            let n = n as usize;
            if self.input.len() < n {
                break;
            }
            let reply = self.server.handle(&self.input[..n]);
            self.output.extend(reply);
            self.input.drain(..n);
        }
        Ok(())
    }
}

impl Device for H9Fs {
    fn name(&self) -> &'static str {
        "h9fs"
    }

    fn read(&mut self, _: u64, _: u8) -> u64 {
        u64::MAX
    }

    fn write(&mut self, _: u64, _: u8, _: u64) {}

    fn call(&mut self, x: &[u64; 6], ram: &Ram) -> Option<u64> {
        Some(match x[0] as u32 {
            h9::FS_RESET => {
                self.server = Server::new(self.opts.clone());
                self.input.clear();
                self.output.clear();
                0
            }
            h9::FS_WRITE => {
                let n = x[2].min(u64::from(MAXMSIZE));
                let mut b = vec![0; n as usize];
                if !ram.read(x[1], &mut b) {
                    return Some(h9::BAD_ADDRESS);
                }
                if self.take(&b).is_err() {
                    return Some(h9::IO_ERROR);
                }
                n
            }
            h9::FS_READ => {
                let n = (x[2] as usize).min(self.output.len());
                if ram.offset(x[1], n).is_none() {
                    return Some(h9::BAD_ADDRESS);
                }
                let b: Vec<u8> = self.output.drain(..n).collect();
                if !ram.write(x[1], &b) {
                    return Some(h9::BAD_ADDRESS);
                }
                n as u64
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::H9Fs;
    use crate::{Bus, h9};
    use h9_9p::Options;
    use h9_9p::msg::{self, NOFID, R, T};
    use h9_vm::Ram;

    #[test]
    fn messages_go_through_the_calls() {
        let root = std::env::temp_dir().join(format!("h9fs-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("hello"), "hello from the host\n").unwrap();
        let ram = Ram::new(0x4000_0000, 0x2_0000).unwrap();
        let mut bus = Bus::new();
        bus.add_hyper(Box::new(H9Fs::new(Options {
            root: root.clone(),
            user: "glenda".into(),
            readonly: false,
        })));
        let mut call = |f: u32, a: u64, b: u64| bus.call(&[u64::from(f), a, b, 0, 0, 0], &ram);
        assert_eq!(call(h9::FS_RESET, 0, 0), 0);
        let mut t = msg::write_t(
            1,
            &T::Version {
                msize: 8192,
                version: "9P2000".into(),
            },
        );
        t.extend(msg::write_t(
            2,
            &T::Attach {
                fid: 0,
                afid: NOFID,
                uname: "glenda".into(),
                aname: String::new(),
            },
        ));
        t.extend(msg::write_t(
            3,
            &T::Walk {
                fid: 0,
                newfid: 1,
                names: vec!["hello".into()],
            },
        ));
        t.extend(msg::write_t(
            4,
            &T::Open {
                fid: 1,
                mode: msg::OREAD,
            },
        ));
        t.extend(msg::write_t(
            5,
            &T::Read {
                fid: 1,
                offset: 0,
                count: 100,
            },
        ));
        assert!(ram.write(0x4000_0000, &t));
        // The messages may come in parts, and a part may end in the middle of a message.
        assert_eq!(call(h9::FS_WRITE, 0x4000_0000, 10), 10);
        assert_eq!(
            call(h9::FS_WRITE, 0x4000_000a, t.len() as u64 - 10),
            t.len() as u64 - 10
        );
        let mut replies = Vec::new();
        loop {
            let n = call(h9::FS_READ, 0x4001_0000, 7);
            if n == 0 {
                break;
            }
            let mut b = vec![0; n as usize];
            assert!(ram.read(0x4001_0000, &mut b));
            replies.extend(b);
        }
        let mut at = 0;
        let mut got = Vec::new();
        while at < replies.len() {
            let n = u32::from_le_bytes(replies[at..at + 4].try_into().unwrap()) as usize;
            got.push(msg::read_r(&replies[at..at + n]).unwrap());
            at += n;
        }
        assert_eq!(got.len(), 5);
        assert_eq!(
            got[4],
            (
                5,
                R::Read {
                    data: b"hello from the host\n".to_vec()
                }
            )
        );
        assert!(ram.write(0x4000_0000, &[3, 0, 0, 0]));
        assert_eq!(call(h9::FS_WRITE, 0x4000_0000, 4), h9::IO_ERROR);
        assert_eq!(call(h9::FS_WRITE, 0x5000_0000, 4), h9::BAD_ADDRESS);
        let _ = std::fs::remove_dir_all(&root);
    }
}
