using System;
using System.Reflection;

public class Base
{
    public virtual string Describe(int value) => "base";
}
public sealed class Derived : Base
{
    public override string Describe(int value)
    {
        GC.Collect();
        return value == 42 ? "derived" : "wrong";
    }
}
public static class Target
{
    public static int Increment(int value) => value + 1;
    public static void Nothing() { }
}
public static class Program
{
    public static int Main()
    {
        object? scalar = typeof(Target).GetMethod(nameof(Target.Increment))!.Invoke(null, new object[] { 41 });
        object? reference = typeof(Base).GetMethod(nameof(Base.Describe))!.Invoke(new Derived(), new object[] { 42 });
        object? empty = typeof(Target).GetMethod(nameof(Target.Nothing))!.Invoke(null, null);
        return scalar is int i && i == 42 && (string?)reference == "derived" && empty == null ? 42 : 1;
    }
}
