using System;

public class Shapes
{
    public string Kind;
    public Shapes() { Kind = "none"; }
    public Shapes(int a) { Kind = "int"; }
    public Shapes(string a) { Kind = "string"; }
    public Shapes(int a, int b) { Kind = "int,int"; }
    private Shapes(double a) { Kind = "private"; }
}
public struct NoCtor { public int X; }
public class OnlyPrivate
{
    private OnlyPrivate() { }
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
        Check(1, ((Shapes)Activator.CreateInstance(typeof(Shapes), new object[] { 1 })!).Kind == "int");
        Check(2, ((Shapes)Activator.CreateInstance(typeof(Shapes), new object[] { "s" })!).Kind == "string");
        Check(3, ((Shapes)Activator.CreateInstance(typeof(Shapes), new object[] { 1, 2 })!).Kind == "int,int");
        Check(4, ((Shapes)Activator.CreateInstance(typeof(Shapes), new object[0])!).Kind == "none");
        Check(5, ((Shapes)Activator.CreateInstance(typeof(Shapes), (object[]?)null)!).Kind == "none");
        Check(6, ((Shapes)Activator.CreateInstance(typeof(Shapes), new object[] { (short)1 })!).Kind == "int");
        Check(7, Throws<MissingMethodException>(() => Activator.CreateInstance(typeof(Shapes), new object[] { 1, 2, 3 })));
        Check(8, Throws<MissingMethodException>(() => Activator.CreateInstance(typeof(Shapes), new object[] { new object() })));
        Check(9, Throws<MissingMethodException>(() => Activator.CreateInstance(typeof(Shapes), new object[] { 1.5 })));
        Check(10, Throws<MissingMethodException>(() => Activator.CreateInstance(typeof(OnlyPrivate), new object[] { 1 })));
        Check(11, Throws<MissingMethodException>(() => Activator.CreateInstance(typeof(OnlyPrivate), new object[0])));
        Check(12, ((NoCtor)Activator.CreateInstance(typeof(NoCtor), new object[0])!).X == 0);
        return failures == 0 ? 42 : 1;
    }
}
