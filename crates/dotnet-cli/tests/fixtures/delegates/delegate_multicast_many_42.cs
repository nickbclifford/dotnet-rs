using System;

public class Program {
    public static int Main() {
        Func<string, string, bool> chain = string.Equals;
        for (int i = 0; i < 100; i++) chain += string.Equals;
        bool result = chain("same", "same");
        Console.WriteLine(result);
        return result ? 42 : 1;
    }
}
