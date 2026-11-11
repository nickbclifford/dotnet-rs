using System;

public ref struct RefValue { public ref int Value; }
public delegate void Consumer(RefValue value);
public delegate RefValue Producer(RefValue value);
public delegate void RefConsumer(ref RefValue value);
public static class Program
{
    static void First(RefValue value) { value.Value += 10; }
    static void Second(RefValue value) { value.Value += 30; }
    static RefValue Identity(RefValue value) => value;
    static void NoOp(ref RefValue value) { }
    public static int Main()
    {
        int number = 2;
        var value = new RefValue { Value = ref number };
        Consumer consume = First; consume += Second;
        Producer produce = Identity; produce += Identity;
        RefConsumer byref = NoOp; byref += NoOp;
        byref(ref value);
        consume(value);
        var result = produce(value);
        Console.WriteLine(result.Value);
        return result.Value == 42 ? 42 : 1;
    }
}
