using System;

public class Program {
    public static void Main() {
        MethodNoParams();
    }

    public static void MethodNoParams() {
        throw new Exception("Test exception");
    }
}
