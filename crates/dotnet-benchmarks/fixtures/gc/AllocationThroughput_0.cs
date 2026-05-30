using System;

public class Program {
    private const int Iterations = 50_000;

    private sealed class Payload {
        public long A;
        public long B;
        public long C;
        public long D;
    }

    public static int Main() {
        long checksum = 0;

        for (int i = 0; i < Iterations; i++) {
            // Keep allocation pressure high without retaining live references.
            var payload = new Payload { A = i, B = i + 3, C = i + 5, D = i + 7 };
            checksum += payload.A + payload.D;
        }

        long expected = (long)Iterations * ((long)Iterations + 6);
        return checksum == expected ? 0 : 1;
    }
}
