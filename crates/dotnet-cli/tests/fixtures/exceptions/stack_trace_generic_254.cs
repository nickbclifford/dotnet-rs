using System;

public class Program {
    public static void Main() {
        GenericMethod<int>(42);
    }

    public static void GenericMethod<T>(T val) {
        throw new Exception("Test exception");
    }
}
