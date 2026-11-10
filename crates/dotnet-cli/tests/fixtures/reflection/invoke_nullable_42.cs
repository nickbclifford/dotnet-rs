using System;
using System.Reflection;

public struct Holder
{
    public int? Value;
    public Holder(int? value) { Value = value; }
}
public static class Target
{
    public static int Unwrap(int? value) => value.HasValue ? value.Value : -1;
    public static int? Wrap(int value) => value == 0 ? null : value;
    public static int Two(int? a, long? b) => (a ?? 1) + (int)(b ?? 10);
}
public static class Program
{
    static int failures;
    static void Check(int id, bool ok)
    {
        if (!ok) { failures++; Console.WriteLine("FAIL " + id); }
    }
    static bool Throws<T>(Action action) where T : Exception
    {
        try { action(); return false; }
        catch (T) { return true; }
        catch (Exception) { return false; }
    }
    public static int Main()
    {
        var unwrap = typeof(Target).GetMethod(nameof(Target.Unwrap))!;
        var wrap = typeof(Target).GetMethod(nameof(Target.Wrap))!;
        var two = typeof(Target).GetMethod(nameof(Target.Two))!;
        Check(1, (int)unwrap.Invoke(null, new object?[] { null })! == -1);
        Check(2, (int)unwrap.Invoke(null, new object?[] { 5 })! == 5);
        Check(3, Throws<ArgumentException>(() => unwrap.Invoke(null, new object[] { "x" })));
        Check(4, wrap.Invoke(null, new object[] { 0 }) == null);
        Check(5, (int)wrap.Invoke(null, new object[] { 9 })! == 9);
        Check(6, (int)two.Invoke(null, new object?[] { null, null })! == 11);
        Check(7, (int)two.Invoke(null, new object?[] { 2, 3L })! == 5);
        var ctor = typeof(Holder).GetConstructor(new[] { typeof(int?) })!;
        Check(8, ((Holder)ctor.Invoke(new object?[] { null })).Value == null);
        Check(9, ((Holder)ctor.Invoke(new object?[] { 4 })).Value == 4);
        var created = (Holder)Activator.CreateInstance(typeof(Holder), new object?[] { null })!;
        Check(10, created.Value == null);
        Check(11, ((Holder)Activator.CreateInstance(typeof(Holder), new object?[] { 6 })!).Value == 6);
        return failures == 0 ? 42 : 1;
    }
}
