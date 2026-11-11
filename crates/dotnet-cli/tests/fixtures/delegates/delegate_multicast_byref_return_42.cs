using System;

public delegate ref int Selector();
public static class Program {
    static int first = 1, last = 40, calls;
    static ref int First() { calls = calls * 10 + 1; return ref first; }
    static ref int Last() { calls = calls * 10 + 2; return ref last; }
    public static int Main() {
        Selector select = First;
        select += Last;
        ref int result = ref select();
        result += 2;
        Console.WriteLine($"{calls} {first} {last}");
        return calls == 12 && first == 1 && last == 42 ? 42 : 1;
    }
}
