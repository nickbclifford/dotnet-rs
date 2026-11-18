using System;

// Exit-only smoke for this known CLR/VM divergence. It does not assert stack-trace behavior.
public class Program {
    public static int Main() {
        try {
            GC.Collect(-1);
        } catch (ArgumentOutOfRangeException) {
            // The CLR rejects the count; this smoke only requires the guest to keep running.
        }
        return 42;
    }
}
