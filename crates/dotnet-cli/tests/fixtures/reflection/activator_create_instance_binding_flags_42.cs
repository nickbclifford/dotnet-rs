using System;
using System.Globalization;
using System.Reflection;

// Activator.CreateInstance overloads that take BindingFlags all funnel into
// RuntimeType.CreateInstanceImpl. Its CoreLib body reads RuntimeType.m_cache, which the VM's
// RuntimeType doesn't have, and used to raise AccessViolationException (plan 24 slice 2).
public class Shapes
{
    public string Kind;
    public Shapes() { Kind = "none"; }
    public Shapes(int a) { Kind = "int"; }
    public Shapes(string a) { Kind = "string"; }
    private Shapes(double a) { Kind = "private"; }
}
public class Params
{
    public int Count;
    public Params(string head, params int[] rest) { Count = rest.Length; }
}
public class Hidden<T>
{
    public T Value;
    internal Hidden(T value) { Value = value; }
}
public struct Point
{
    public int X;
    public Point(int x) { X = x; }
}
public static class Program
{
    const BindingFlags PublicInstance = BindingFlags.Instance | BindingFlags.Public;
    const BindingFlags AnyInstance = BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic;

    static int failures;
    static void Check(int id, bool ok)
    {
        if (!ok) { failures++; Console.WriteLine("FAIL " + id); }
    }
    static bool Throws<T>(Action action) where T : Exception
    {
        try { action(); return false; }
        catch (T) { return true; }
        catch (Exception e) { Console.WriteLine(e.GetType().FullName); return false; }
    }
    static object? Create(Type type, BindingFlags flags, params object?[]? args) =>
        Activator.CreateInstance(type, flags, null, args, CultureInfo.InvariantCulture);

    public static int Main()
    {
        Check(1, ((Shapes)Create(typeof(Shapes), PublicInstance, 1)!).Kind == "int");
        Check(2, ((Shapes)Create(typeof(Shapes), PublicInstance, "s")!).Kind == "string");
        Check(3, ((Shapes)Create(typeof(Shapes), PublicInstance)!).Kind == "none");
        Check(4, ((Shapes)Create(typeof(Shapes), AnyInstance, 1.5)!).Kind == "private");
        Check(5, Throws<MissingMethodException>(() => Create(typeof(Shapes), PublicInstance, 1.5)));
        Check(6, Throws<MissingMethodException>(() => Create(typeof(Shapes), PublicInstance, 1, 2, 3)));

        // System.Text.Json's ObjectConverterFactory shape: a MakeGenericType'd type with a
        // non-public constructor.
        var hidden = typeof(Hidden<>).MakeGenericType(typeof(string));
        var made = Create(hidden, AnyInstance, "v")!;
        Check(7, made is Hidden<string> h && h.Value == "v");
        Check(8, Throws<MissingMethodException>(() => Create(hidden, PublicInstance, "v")));

        // params arrays bind only when CreateInstance (or InvokeMethod etc.) is in the flags. The
        // positive cases need ParamArrayAttribute on ParameterInfo, which the VM doesn't report
        // yet (plans README "Not yet planned").
        Check(11, Throws<MissingMethodException>(() => Create(typeof(Params), PublicInstance, "h", 1, 2)));

        // Value types: no arguments gives the default value; arguments bind a constructor.
        Check(12, ((Point)Create(typeof(Point), PublicInstance)!).X == 0);
        Check(13, ((Point)Create(typeof(Point), PublicInstance, 7)!).X == 7);

        Check(14, Throws<ArgumentException>(() => Create(typeof(Hidden<>), AnyInstance, "v")));
        Check(15, Throws<TargetInvocationException>(() =>
            Activator.CreateInstance(typeof(Thrower), PublicInstance, null, new object[] { 1 }, null)));
        Check(16, Throws<InvalidOperationException>(() =>
            Activator.CreateInstance(typeof(Thrower), PublicInstance | BindingFlags.DoNotWrapExceptions, null, new object[] { 1 }, null)));

        Console.WriteLine(failures == 0 ? "ok" : "failures: " + failures);
        return failures == 0 ? 42 : 1;
    }
}
public class Thrower
{
    public Thrower(int x) { throw new InvalidOperationException("boom"); }
}
