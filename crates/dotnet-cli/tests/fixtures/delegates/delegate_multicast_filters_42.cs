using System;

public delegate int Work(int value);
public class Program {
    static int count;
    static int First(int value) { Console.WriteLine("first"); return value; }
    static int Throwing(int value) {
        try { Console.WriteLine("throw"); throw new InvalidOperationException(); }
        finally { Console.WriteLine("finally"); }
    }
    static int Never(int value) { Console.WriteLine("never"); count = -100; return value; }
    static bool InnerFilter(Exception exception) { Console.WriteLine("inner filter"); return exception is InvalidOperationException; }
    static int Handled(int value) {
        try { throw new InvalidOperationException(); }
        catch (Exception exception) when (InnerFilter(exception)) { Console.WriteLine("inner catch"); return value + 1; }
    }
    static int Last(int value) { Console.WriteLine("last"); return value + 2; }
    static int Nested(int value) {
        Work inner = Handled;
        inner += Last;
        return inner(value);
    }
    static bool Filter(Exception exception) {
        Console.WriteLine("filter");
        Work inner = Handled;
        inner += Last;
        count += inner(10);
        return exception is InvalidOperationException;
    }
    public static int Main() {
        Work chain = First;
        chain += Throwing;
        chain += Never;
        try { chain(0); }
        catch (Exception exception) when (Filter(exception)) { Console.WriteLine("outer catch"); }
        Work nested = First;
        nested += Nested;
        count += nested(28);
        Console.WriteLine(count);
        return count == 42 ? 42 : 1;
    }
}
