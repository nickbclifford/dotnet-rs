using System;
using System.Reflection;

public class Ctors
{
    public string Kind;
    public Ctors() { Kind = "none"; }
    public Ctors(int a) { Kind = "int"; }
    public Ctors(string a) { Kind = "string"; }
    public Ctors(int a, string b) { Kind = "int,string"; }
    private Ctors(double a) { Kind = "private"; }
}
public static class Program
{
    static int failures;
    static void Check(int id, bool ok)
    {
        if (!ok) { failures++; Console.WriteLine("FAIL " + id); }
    }
    static string Kind(ConstructorInfo c, params object[] args) => ((Ctors)c.Invoke(args)).Kind;
    public static int Main()
    {
        var t = typeof(Ctors);
        Check(1, Kind(t.GetConstructor(Type.EmptyTypes)!) == "none");
        Check(2, Kind(t.GetConstructor(new[] { typeof(int) })!, 1) == "int");
        Check(3, Kind(t.GetConstructor(new[] { typeof(string) })!, "s") == "string");
        Check(4, Kind(t.GetConstructor(new[] { typeof(int), typeof(string) })!, 1, "s") == "int,string");
        Check(5, t.GetConstructor(new[] { typeof(long) }) == null);
        Check(6, t.GetConstructor(new[] { typeof(string), typeof(int) }) == null);
        Check(7, t.GetConstructor(new[] { typeof(double) }) == null);
        Check(8, t.GetConstructor(new[] { typeof(int), typeof(int), typeof(int) }) == null);
        Check(9, t.GetConstructor(BindingFlags.NonPublic | BindingFlags.Instance, null, new[] { typeof(double) }, null) != null);
        Check(10, typeof(object).GetConstructor(new[] { typeof(int) }) == null);
        Check(11, typeof(object).GetConstructor(Type.EmptyTypes) != null);
        Check(12, t.GetConstructor(new[] { typeof(int) })!.GetParameters().Length == 1);
        return failures == 0 ? 42 : 1;
    }
}
