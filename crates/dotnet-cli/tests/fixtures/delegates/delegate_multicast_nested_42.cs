using System;

public delegate int Work(int value);
public delegate void RefWork(ref int value);
public static class LaterNested {
    static LaterNested() { Console.WriteLine("nested cctor"); }
    public static int Second(int value) { Console.WriteLine("nested target"); return value + 2; }
}
public class Program {
    static int storage;
    static int First(int value) { Console.WriteLine("first"); return value + 1; }
    static int Second(int value) { Console.WriteLine("second"); return value + 2; }
    static void RefFirst(ref int value) { Console.WriteLine("ref first"); value = 10; }
    static void RefSecond(ref int value) { Console.WriteLine("ref second"); value += 11; }
    static int CallsFallback(int value) {
        RefWork fallback = RefFirst;
        fallback += RefSecond;
        fallback(ref storage);
        return value + storage;
    }
    static void CallsAsync(ref int value) {
        Work inner = First;
        inner += Second;
        value += inner(19);
    }
    public static int Main() {
        Work inner = First;
        inner += Second;
        Work outer = First;
        outer += new Work(inner.Invoke);
        int one = outer(19);
        Work asyncToFallback = First;
        asyncToFallback += CallsFallback;
        int two = asyncToFallback(0);
        RefWork fallbackToAsync = RefFirst;
        fallbackToAsync += CallsAsync;
        fallbackToAsync(ref storage);
        Work single = LaterNested.Second;
        Work wrappingSingle = First;
        wrappingSingle += new Work(single.Invoke);
        int delayed = wrappingSingle(40);
        Console.WriteLine(one + "," + two + "," + storage + "," + delayed);
        return one == 21 && two == 21 && storage == 31 && delayed == 42 ? 42 : 1;
    }
}
