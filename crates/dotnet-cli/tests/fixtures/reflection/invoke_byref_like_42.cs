using System;
using System.IO;
using System.Reflection;

// Reflection cannot box byref-like values, so the CLR refuses them as arguments and results
// with NotSupportedException before the target runs. Activator.CreateInstance(Type, object[])
// raises its own argument and binding exceptions, and creates a default struct when no
// constructor matches an empty argument list.
public ref struct Plain { public int X; }
public ref struct WithRef { public ref int X; }
public struct Value { public int X; public Value(int x) { X = x; } }
public class NoDefault { public NoDefault(int x) { } }
public abstract class Abstract { public Abstract() { } }

public static class Program
{
    static int calls;
    public static void TakePlain(Plain value) { calls++; }
    public static void TakeWithRef(WithRef value) { calls++; }
    public static void TakeSpan(Span<int> value) { calls++; }
    public static void TakeRefSpan(ref Span<int> value) { calls++; }
    public static void TakeTypedReference(TypedReference value) { calls++; }
    public static void TakeIntThenSpan(int first, Span<int> second) { calls++; }
    public static Plain ReturnPlain() { calls++; return default; }
    public static Span<int> ReturnSpan() { calls++; return default; }

    static string Run(Func<object> body, bool message)
    {
        try
        {
            object result = body();
            return result switch
            {
                null => "null",
                Value value => $"Value({value.X})",
                int number => $"int({number})",
                _ => result.GetType().Name,
            };
        }
        catch (Exception e)
        {
            return message ? $"{e.GetType().FullName}: {e.Message}" : e.GetType().FullName!;
        }
    }

    static object Invoke(string name, params object[] args) =>
        typeof(Program).GetMethod(name)!.Invoke(null, args)!;

    public static int Main()
    {
        string[] lines =
        {
            Run(() => Invoke(nameof(TakePlain), new object[] { null! }), true),
            Run(() => Invoke(nameof(TakeWithRef), new object[] { null! }), true),
            Run(() => Invoke(nameof(TakeSpan), new object[] { null! }), true),
            Run(() => Invoke(nameof(TakeRefSpan), new object[] { null! }), true),
            Run(() => Invoke(nameof(TakeTypedReference), new object[] { null! }), true),
            Run(() => Invoke(nameof(TakeIntThenSpan), 1, null!), true),
            Run(() => Invoke(nameof(TakeSpan)), true),
            Run(() => Invoke(nameof(ReturnPlain)), true),
            Run(() => Invoke(nameof(ReturnSpan)), true),
            Run(() => Activator.CreateInstance(typeof(Value), Array.Empty<object>()), true),
            Run(() => Activator.CreateInstance(typeof(Value), (object[])null!), true),
            Run(() => Activator.CreateInstance(typeof(Value), new object[] { 7 }), true),
            Run(() => Activator.CreateInstance(typeof(Value), new object[] { "x" }), true),
            Run(() => Activator.CreateInstance(typeof(int), Array.Empty<object>()), true),
            Run(() => Activator.CreateInstance(typeof(int), new object[] { 1 }), true),
            Run(() => Activator.CreateInstance(typeof(NoDefault), Array.Empty<object>()), true),
            Run(() => Activator.CreateInstance(typeof(NoDefault), new object[] { "s" }), true),
            Run(() => Activator.CreateInstance(typeof(Abstract), Array.Empty<object>()), true),
            Run(() => Activator.CreateInstance(typeof(Stream), Array.Empty<object>()), true),
            Run(() => Activator.CreateInstance(typeof(IDisposable), Array.Empty<object>()), true),
            Run(() => Activator.CreateInstance(null!, Array.Empty<object>()), true),
            Run(() => Activator.CreateInstance(typeof(Plain), Array.Empty<object>()), true),
            Run(() => Activator.CreateInstance(typeof(Span<int>), Array.Empty<object>()), true),
            Run(() => Activator.CreateInstance(typeof(object), Array.Empty<object>()), true),
            Run(() => Activator.CreateInstance(typeof(TypedReference), Array.Empty<object>()), true),
            Run(() => Activator.CreateInstance(typeof(int[]), Array.Empty<object>()), false),
            Run(() => Activator.CreateInstance(typeof(System.Collections.Generic.List<>), Array.Empty<object>()), false),
        };
        foreach (string line in lines)
        {
            Console.WriteLine(line);
        }
        Console.WriteLine($"calls: {calls}");
        return calls == 0 ? 42 : 1;
    }
}
