using System;
using System.Reflection;
using System.Runtime.CompilerServices;

public class Program
{
    static int number = 17;
    static string text = "hello";
    static int[] array = { 23 };
    int field = 31;
    public static ref int StaticValue() => ref number;
    public static ref string StaticReference() => ref text;
    public static ref int Element() => ref array[0];
    public ref int Field() => ref field;
    public static ref Span<int> ByrefLike() => ref Unsafe.NullRef<Span<int>>();
    public static ref int Null() => ref Unsafe.NullRef<int>();

    static object? Invoke(string name, object? target = null) =>
        typeof(Program).GetMethod(name)!.Invoke(target, null);

    public static int Main()
    {
        if ((int)Invoke(nameof(StaticValue))! != 17) return 1;
        if ((string)Invoke(nameof(StaticReference))! != "hello") return 2;
        if ((int)Invoke(nameof(Element))! != 23) return 3;
        if ((int)Invoke(nameof(Field), new Program())! != 31) return 4;
        try { Invoke(nameof(ByrefLike)); return 5; }
        catch (NotSupportedException) { }
        try { Invoke(nameof(Null)); return 6; }
        catch (TargetInvocationException e) when (e.InnerException is NullReferenceException) { }
        try {
            typeof(Program).GetMethod(nameof(Null))!.Invoke(null, BindingFlags.DoNotWrapExceptions, null, null, null);
            return 7;
        } catch (NullReferenceException) { }
        return 42;
    }
}
