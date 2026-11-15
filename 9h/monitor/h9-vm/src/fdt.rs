//! A writer of flattened device trees, version 17. The arm64 kernel reads the size of the RAM,
//! the processors and the boot arguments from it (`bootargs.c`).

/// A device tree that is being written.
#[derive(Default)]
pub struct Fdt {
    structs: Vec<u8>,
    strings: Vec<u8>,
    depth: usize,
}

const BEGIN_NODE: u32 = 1;
const END_NODE: u32 = 2;
const PROP: u32 = 3;
const END: u32 = 9;
const MAGIC: u32 = 0xd00d_feed;

impl Fdt {
    /// A tree with no nodes.
    #[must_use]
    pub fn new() -> Fdt {
        Fdt::default()
    }

    fn word(&mut self, w: u32) {
        self.structs.extend_from_slice(&w.to_be_bytes());
    }

    fn pad(&mut self) {
        while !self.structs.len().is_multiple_of(4) {
            self.structs.push(0);
        }
    }

    /// Start a node. The first node is the root, with the empty name.
    pub fn begin(&mut self, name: &str) {
        self.word(BEGIN_NODE);
        self.structs.extend_from_slice(name.as_bytes());
        self.structs.push(0);
        self.pad();
        self.depth += 1;
    }

    /// End the last node that was started.
    pub fn end(&mut self) {
        assert!(self.depth > 0, "fdt: end with no node");
        self.word(END_NODE);
        self.depth -= 1;
    }

    fn name(&mut self, name: &str) -> u32 {
        let mut at = 0;
        for s in self.strings.split(|&b| b == 0) {
            if s == name.as_bytes() {
                return at as u32;
            }
            at += s.len() + 1;
        }
        let at = self.strings.len() as u32;
        self.strings.extend_from_slice(name.as_bytes());
        self.strings.push(0);
        at
    }

    /// A property with these bytes.
    pub fn prop(&mut self, name: &str, value: &[u8]) {
        let off = self.name(name);
        self.word(PROP);
        self.word(value.len() as u32);
        self.word(off);
        self.structs.extend_from_slice(value);
        self.pad();
    }

    /// A property with a string, which gets its NUL.
    pub fn string(&mut self, name: &str, value: &str) {
        let mut v = value.as_bytes().to_vec();
        v.push(0);
        self.prop(name, &v);
    }

    /// A property with 32-bit cells.
    pub fn cells(&mut self, name: &str, cells: &[u32]) {
        let v: Vec<u8> = cells.iter().flat_map(|c| c.to_be_bytes()).collect();
        self.prop(name, &v);
    }

    /// A property with 64-bit values, each as two cells.
    pub fn cells64(&mut self, name: &str, values: &[u64]) {
        let v: Vec<u8> = values.iter().flat_map(|c| c.to_be_bytes()).collect();
        self.prop(name, &v);
    }

    /// The bytes of the tree. Each node must be ended.
    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        assert_eq!(self.depth, 0, "fdt: a node is not ended");
        self.word(END);
        let rsvmap = 40u32;
        let structs = rsvmap + 16;
        let strings = structs + self.structs.len() as u32;
        let total = strings + self.strings.len() as u32;
        let mut out = Vec::with_capacity(total as usize);
        for w in [
            MAGIC,
            total,
            structs,
            strings,
            rsvmap,
            17,
            16,
            0,
            self.strings.len() as u32,
            self.structs.len() as u32,
        ] {
            out.extend_from_slice(&w.to_be_bytes());
        }
        out.extend_from_slice(&[0; 16]);
        out.extend_from_slice(&self.structs);
        out.extend_from_slice(&self.strings);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Fdt;

    fn be32(b: &[u8], at: usize) -> u32 {
        u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn the_header_points_at_the_blocks() {
        let mut t = Fdt::new();
        t.begin("");
        t.cells("#address-cells", &[2]);
        t.begin("chosen");
        t.string("bootargs", "console=0");
        t.end();
        t.begin("memory@40000000");
        t.cells64("reg", &[0x4000_0000, 0x4000_0000]);
        t.cells("#address-cells", &[2]);
        t.end();
        t.end();
        let b = t.finish();
        assert_eq!(be32(&b, 0), 0xd00d_feed);
        assert_eq!(be32(&b, 4) as usize, b.len());
        let structs = be32(&b, 8) as usize;
        let strings = be32(&b, 12) as usize;
        assert_eq!(be32(&b, structs), 1);
        assert_eq!(&b[strings..], b"#address-cells\0bootargs\0reg\0");
        assert_eq!(be32(&b, strings - 4), 9);
    }
}
