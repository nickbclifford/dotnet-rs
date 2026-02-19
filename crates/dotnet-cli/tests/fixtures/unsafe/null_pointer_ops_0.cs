using System;
using System.Runtime.CompilerServices;

public class Program {
    public static int Main() {
        bool caughtCpblk = false;
        try {
            unsafe {
                Unsafe.CopyBlock(null, null, 1);
            }
        } catch (NullReferenceException) {
            caughtCpblk = true;
        }

        if (!caughtCpblk) return 1;

        bool caughtInitblk = false;
        try {
            unsafe {
                Unsafe.InitBlock(null, 0, 1);
            }
        } catch (NullReferenceException) {
            caughtInitblk = true;
        }

        if (!caughtInitblk) return 2;

        return 0;
    }
}
