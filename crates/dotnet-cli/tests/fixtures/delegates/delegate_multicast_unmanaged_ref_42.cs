using System;
using System.Threading;

public delegate void Update(ref int value);
public delegate void SetFlag(ref bool value);
public static class Program {
    static void First(ref int value) { value += 10; }
    static void Second(ref int value) { value += 30; }
    static object gate = new object();
    static void Set(ref bool value) { Monitor.Enter(gate, ref value); }
    static void Check(ref bool value) { if (!value) throw new Exception(); Monitor.Exit(gate); }
    public static unsafe int Main() {
        int number = 2;
        int* pointer = &number;
        Update update = First;
        update += Second;
        update(ref *pointer);
        bool flag = false;
        bool* flagPointer = &flag;
        SetFlag setFlag = Set;
        setFlag += Check;
        setFlag(ref *flagPointer);
        flag = false;
        Monitor.TryEnter(gate, 0, ref *flagPointer);
        if (!flag) return 2;
        Monitor.Exit(gate);
        Console.WriteLine(number);
        return number == 42 && flag ? 42 : 1;
    }
}
