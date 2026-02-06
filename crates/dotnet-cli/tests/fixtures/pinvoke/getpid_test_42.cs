using System;
using System.Runtime.InteropServices;

public class Program {
    [DllImport("libc")]
    public static extern int getpid();

    public static int Main() {
        int pid = getpid();
        if (pid > 0) {
            return 42;
        }
        return pid;
    }
}
