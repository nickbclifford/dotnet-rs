using System;

public class Program {
    public static int Main() {
        int result = 0;
        Action<int> setter = x => result = x;
        setter(42);
        if (result == 42) return 0;
        return 1;
    }
}
