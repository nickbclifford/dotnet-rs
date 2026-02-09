using System;

public class Program {
    delegate int BinaryOp(int a, int b);
    
    public static int Add(int a, int b) => a + b;
    
    public static int Main() {
        BinaryOp op = Add;
        return op(20, 22); // Should return 42
    }
}
