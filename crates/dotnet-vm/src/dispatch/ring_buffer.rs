use gc_arena::{Collect, unsafe_empty_collect};

#[derive(Default, Clone)]
pub struct InstructionRingBuffer {
    // Store (IP, Formatted Instruction) to provide more context on timeout/panic
    buffer: [Option<(usize, String)>; 10],
    index: usize,
}

unsafe_empty_collect!(InstructionRingBuffer);

impl InstructionRingBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, ip: usize, instruction: String) {
        self.buffer[self.index] = Some((ip, instruction));
        self.index = (self.index + 1) % 10;
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
            buffer.push(i, "Add".to_string());
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
