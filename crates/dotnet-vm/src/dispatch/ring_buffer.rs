// AI-GENERATED FILE
use gc_arena::Collect;

#[derive(Default, Collect, Clone)]
#[collect(require_static)]
pub struct InstructionRingBuffer {
    buffer: [Option<(usize, String)>; 10],
    index: usize,
}

impl InstructionRingBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, ip: usize, text: String) {
        self.buffer[self.index] = Some((ip, text));
        self.index = (self.index + 1) % 10;
    }

    pub fn dump(&self) -> String {
        let mut res = String::new();
        // The instructions are pushed into the buffer at `index`.
        // To dump them in chronological order, we start from the oldest instruction.
        // If the buffer is not full, the oldest is at index 0.
        // If the buffer is full, the oldest is at `index`.
        
        // Simpler way: just iterate 10 times from (index) to (index + 9) % 10
        for i in 0..10 {
            let idx = (self.index + i) % 10;
            if let Some((ip, text)) = &self.buffer[idx] {
                res.push_str(&format!("[{:04x}] {}
", ip, text));
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
            buffer.push(i, format!("instr {}", i));
        }
        
        let dump = buffer.dump();
        // Should contain instrs 5 to 14
        assert!(!dump.contains("instr 4"));
        assert!(dump.contains("instr 5"));
        assert!(dump.contains("instr 14"));
        
        let lines: Vec<_> = dump.lines().collect();
        assert_eq!(lines.len(), 10);
        assert!(lines[0].contains("instr 5"));
        assert!(lines[9].contains("instr 14"));
    }
}
