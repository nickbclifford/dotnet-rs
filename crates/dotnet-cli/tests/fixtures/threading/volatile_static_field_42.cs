using System;
using System.Threading;

public class Program {
    static volatile int s_vol;
    
    public static int Main() {
        s_vol = 42;
        int val = s_vol;
        if (val == 42) {
            return 42;
        }
        return 0;
    }
}
