using System;

public delegate void RefWork(ref int value);
public class Program {
    static int value;
    static void First(ref int target) { Console.WriteLine("first"); target = 10; }
    static void Throwing(ref int target) {
        try { Console.WriteLine("throw"); target += 2; throw new InvalidOperationException(); }
        finally { Console.WriteLine("finally"); target += 10; }
    }
    static void Never(ref int target) { Console.WriteLine("never"); target = 0; }
    static int filterValue;
    static void FilterFirst(ref int target) { target = 1; }
    static bool NestedFilter(Exception exception) { Console.WriteLine("nested filter"); return exception is InvalidOperationException; }
    static void FilterSecond(ref int target) {
        try { throw new InvalidOperationException(); }
        catch (Exception exception) when (NestedFilter(exception)) { target += 1; }
    }
    static bool Filter(Exception exception) {
        Console.WriteLine("filter " + value);
        RefWork filter = FilterFirst;
        filter += FilterSecond;
        filter(ref filterValue);
        Console.WriteLine("filter result " + filterValue);
        return exception is InvalidOperationException && filterValue == 2;
    }
    public static int Main() {
        RefWork chain = First;
        chain += Throwing;
        chain += Never;
        try { chain(ref value); }
        catch (Exception exception) when (Filter(exception)) { Console.WriteLine("caught"); value += 20; }
        Console.WriteLine(value);
        return value == 42 ? 42 : 1;
    }
}
