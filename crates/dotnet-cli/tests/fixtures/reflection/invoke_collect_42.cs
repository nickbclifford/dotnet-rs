using System;
using System.Reflection;

public class Program {
    public static int Main() {
        var none = typeof(GC).GetMethod("Collect", Type.EmptyTypes)!;
        Console.WriteLine(none.Invoke(null, null) == null);
        var generation = typeof(GC).GetMethod("Collect", new[] { typeof(int) })!;
        Console.WriteLine(generation.Invoke(null, new object[] { 0 }) == null);
        var mode = typeof(GC).GetMethod("Collect", new[] { typeof(int), typeof(GCCollectionMode) })!;
        Console.WriteLine(mode.Invoke(null, new object[] { 0, GCCollectionMode.Forced }) == null);
        Console.WriteLine("done");
        return 42;
    }
}
