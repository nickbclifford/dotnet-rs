using System;

public ref struct RefValue { public ref int Value; }
public delegate void Consumer(RefValue value);
public delegate RefValue Producer(RefValue value);
public delegate void RefConsumer(ref RefValue value);
public static class Program
{
    static int other = 7;
    static void First(RefValue value) { value.Value += 10; GC.Collect(); }
    static void Second(RefValue value) { value.Value += 30; }
    static RefValue Other(RefValue value) => new RefValue { Value = ref other };
    static RefValue Identity(RefValue value) { GC.Collect(); return value; }
    static void Redirect(ref RefValue value) { value.Value = ref other; GC.Collect(); }
    static void RefAdd(ref RefValue value) { value.Value++; GC.Collect(); }
    public static int Main()
    {
        int number = 2;
        var value = new RefValue { Value = ref number };
        Consumer consume = First; consume += Second;
        Producer produce = Other; produce += Identity;
        RefConsumer byref = Redirect; byref += RefAdd;
        byref(ref value);
        if (other != 8 || value.Value != 8 || number != 2) return 2;
        value.Value = ref number;
        consume(value);
        var result = produce(value);
        Console.WriteLine(result.Value);
        result.Value++;
        return number == 43 && other == 8 ? 42 : 1;
    }
}
