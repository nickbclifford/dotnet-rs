using System.Threading;

public class Program {
    private static int NextRole;
    public static volatile int ReaderReady = 0;
    public static volatile int Flag = 0;
    public static int Data = 0;

    public static int Main() {
        int role = Interlocked.Increment(ref NextRole);
        if (role == 1) {
            for (int spin = 0; spin < 100000 && ReaderReady != 1; spin++) Thread.Yield();
            if (ReaderReady != 1) return 1;
            Data = 42;
            Flag = 1;
        } else if (role == 2) {
            ReaderReady = 1;
            for (int spin = 0; spin < 100000 && Flag != 1; spin++) Thread.Yield();
            if (Flag != 1) return 2;
            if (Data != 42) return 3;
        } else {
            return 3;
        }
        return 42;
    }
}
