using System;

public delegate ref int Selector();
public static class Program {
    sealed class Holder { public int value = 40; }
    static int first = 1, calls;
    static ref int First() { calls = calls * 10 + 1; return ref first; }
    static ref int Last() { calls = calls * 10 + 2; GC.Collect(); return ref new Holder().value; }
    public static int Main() {
        Selector select = First;
        select += Last;
        ref int result = ref select();
        GC.Collect();
        result += 2;
        Console.WriteLine($"{calls} {first} {result}");
        return calls == 12 && first == 1 && result == 42 ? 42 : 1;
    }
}
