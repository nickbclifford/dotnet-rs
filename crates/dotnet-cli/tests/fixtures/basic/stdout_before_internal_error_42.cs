using System;

public class Program {
    public static int Main() {
        Console.Write("stdout ");
        Console.Write("written ");
        Console.WriteLine("before return");
        Console.Write("without trailing newline");
        return 42;
    }
}
