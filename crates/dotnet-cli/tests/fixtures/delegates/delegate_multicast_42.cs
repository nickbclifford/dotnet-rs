using System;

public class Program {
    public static int Main() {
        int result = 0;
        Action<int> setter = x => result = x;
        setter += x => result += x;
        setter(21); // Sets to 21, then adds 21 = 42
        return result;
    }
}
