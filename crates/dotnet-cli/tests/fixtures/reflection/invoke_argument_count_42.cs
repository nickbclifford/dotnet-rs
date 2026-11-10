using System;
using System.Reflection;

public class Counted
{
    public int Sum;
    public Counted() { Sum = -1; }
    public Counted(int a, int b) { Sum = a + b; }
    public static int Add(int a, int b) => a + b;
    public static int Zero() => 0;
    public int Instance(int a) => a;
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
        var add = typeof(Counted).GetMethod(nameof(Counted.Add))!;
        var zero = typeof(Counted).GetMethod(nameof(Counted.Zero))!;
        var instance = typeof(Counted).GetMethod(nameof(Counted.Instance))!;
        var ctor = typeof(Counted).GetConstructor(new[] { typeof(int), typeof(int) })!;
        var defaultCtor = typeof(Counted).GetConstructor(Type.EmptyTypes)!;

        Check(1, (int)add.Invoke(null, new object[] { 1, 2 })! == 3);
        Check(2, Throws<TargetParameterCountException>(() => add.Invoke(null, new object[] { 1 })));
        Check(3, Throws<TargetParameterCountException>(() => add.Invoke(null, new object[] { 1, 2, 3 })));
        Check(4, Throws<TargetParameterCountException>(() => add.Invoke(null, null)));
        Check(5, Throws<TargetParameterCountException>(() => add.Invoke(null, new object[0])));
        Check(6, (int)zero.Invoke(null, null)! == 0);
        Check(7, (int)zero.Invoke(null, new object[0])! == 0);
        Check(8, Throws<TargetParameterCountException>(() => zero.Invoke(null, new object[] { 1 })));
        Check(9, Throws<TargetParameterCountException>(() => instance.Invoke(new Counted(), new object[0])));
        Check(10, (int)instance.Invoke(new Counted(), new object[] { 7 })! == 7);
        Check(11, Throws<TargetParameterCountException>(() => ctor.Invoke(new object[] { 1 })));
        Check(12, Throws<TargetParameterCountException>(() => ctor.Invoke(null)));
        Check(13, Throws<TargetParameterCountException>(() => defaultCtor.Invoke(new object[] { 1 })));
        Check(14, ((Counted)ctor.Invoke(new object[] { 3, 4 })).Sum == 7);
        Check(15, ((Counted)defaultCtor.Invoke(null)).Sum == -1);
        Check(16, ((Counted)defaultCtor.Invoke(new object[0])).Sum == -1);
        return failures == 0 ? 42 : 1;
    }
}
