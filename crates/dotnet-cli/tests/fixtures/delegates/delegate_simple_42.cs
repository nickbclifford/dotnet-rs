using System;

public class Program {
    delegate int SimpleOp(int x);
    
    public static int Main() {
        SimpleOp doubler = x => x * 2;
        return doubler(21); // Should return 42
    }
}
