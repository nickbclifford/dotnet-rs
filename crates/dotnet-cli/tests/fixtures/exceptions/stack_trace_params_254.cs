using System;

public class Program {
    public static void Main(string[] args) {
        MethodWithParams("hello", 42);
    }

    public static void MethodWithParams(string s, int i) {
        throw new Exception("Test exception");
    }
}
