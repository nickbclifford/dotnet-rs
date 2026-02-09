using System;

public class Program {
    public static int Main() {
        Func<int, int> doubler = x => x * 2;
        return doubler(21); // Should return 42
    }
}
