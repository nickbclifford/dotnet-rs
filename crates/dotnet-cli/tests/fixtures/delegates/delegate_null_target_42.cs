using System;

public class Program {
    delegate int UnaryOp(int x);
    
    public static int Identity(int x) => x;
    
    public static int Main() {
        UnaryOp op = Identity;
        return op(42); // Should return 42
    }
}
