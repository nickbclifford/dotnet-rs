using System.Threading;

public static class Program {
    public static int Main() {
        Thread current = Thread.CurrentThread;
        if (current.IsBackground) return 1;
        current.IsBackground = true;
        if (!Thread.CurrentThread.IsBackground) return 2;
        current.IsBackground = false;
        return current.IsBackground ? 3 : 42;
    }
}
