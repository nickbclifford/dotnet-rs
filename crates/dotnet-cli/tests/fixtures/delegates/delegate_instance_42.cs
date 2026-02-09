using System;

public class Calculator {
    private int offset;
    public Calculator(int offset) { this.offset = offset; }
    public int AddOffset(int x) => x + offset;
}

public class Program {
    delegate int UnaryOp(int x);
    
    public static int Main() {
        var calc = new Calculator(10);
        UnaryOp op = calc.AddOffset;
        return op(32); // Should return 42
    }
}
