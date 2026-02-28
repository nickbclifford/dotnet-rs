use gc_arena::Collect;

#[derive(Default, Collect, Clone)]
#[collect(require_static)]
pub struct InstructionRingBuffer {
    // Store (IP, Instruction Name) to avoid cloning the full Instruction enum
    buffer: [Option<(usize, &'static str)>; 10],
    index: usize,
}

impl InstructionRingBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, ip: usize, name: &'static str) {
        self.buffer[self.index] = Some((ip, name));
        self.index = (self.index + 1) % 10;
    }

    pub fn dump_formatted(&self, _resolution: &dotnet_types::resolution::ResolutionS) -> String {
        let mut res = String::new();
        
        for i in 0..10 {
            let idx = (self.index + i) % 10;
            if let Some((ip, name)) = &self.buffer[idx] {
                res.push_str(&format!(
                    "[{:04x}] {}
",
                    ip, name
                ));
            }
        }
        res
    }

    pub fn dump(&self) -> String {
        let mut res = String::new();
        for i in 0..10 {
            let idx = (self.index + i) % 10;
            if let Some((ip, name)) = &self.buffer[idx] {
                res.push_str(&format!(
                    "[{:04x}] {}
",
                    ip, name
                ));
            }
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_wrapping() {
        let mut buffer = InstructionRingBuffer::new();
        for i in 0..15 {
            buffer.push(i, "Add");
        }

        let dump = buffer.dump();
        // Should contain 10 Adds
        let lines: Vec<_> = dump.lines().collect();
        assert_eq!(lines.len(), 10);
        for line in lines {
            assert!(line.contains("Add"));
        }
    }
}
