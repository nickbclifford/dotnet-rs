using System;
using System.Reflection;

public interface IGreeter { string Greet(); }
public class Person : IGreeter
{
    public string Name = "p";
    public string Hello() => "hello " + Name;
    public virtual string Greet() => "person";
    public static string Stat() => "static";
}
public class Student : Person
{
    public override string Greet() => "student";
}
public class Unrelated { }
public struct Point
{
    public int X;
    public int Get() => X;
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
        var hello = typeof(Person).GetMethod("Hello")!;
        var greet = typeof(Person).GetMethod("Greet")!;
        var stat = typeof(Person).GetMethod("Stat")!;
        var iface = typeof(IGreeter).GetMethod("Greet")!;
        var get = typeof(Point).GetMethod("Get")!;
        Check(1, (string)hello.Invoke(new Person(), null)! == "hello p");
        Check(2, (string)hello.Invoke(new Student(), null)! == "hello p");
        Check(3, Throws<TargetException>(() => hello.Invoke(null, null)));
        Check(4, Throws<TargetException>(() => hello.Invoke(new Unrelated(), null)));
        Check(5, Throws<TargetException>(() => hello.Invoke("string", null)));
        Check(6, Throws<TargetException>(() => hello.Invoke(5, null)));
        Check(7, (string)greet.Invoke(new Student(), null)! == "student");
        Check(8, (string)stat.Invoke(null, null)! == "static");
        Check(9, (string)stat.Invoke(new Unrelated(), null)! == "static");
        Check(10, (string)iface.Invoke(new Person(), null)! == "person");
        Check(11, Throws<TargetException>(() => iface.Invoke(new Unrelated(), null)));
        Check(12, Throws<TargetException>(() => iface.Invoke(null, null)));
        Check(13, (int)get.Invoke(new Point { X = 3 }, null)! == 3);
        Check(14, Throws<TargetException>(() => get.Invoke(null, null)));
        Check(15, Throws<TargetException>(() => get.Invoke(new Person(), null)));
        Check(16, Throws<TargetException>(() => typeof(object).GetMethod("ToString")!.Invoke(null, null)));
        Check(17, (string)typeof(object).GetMethod("ToString")!.Invoke(new Person(), null)! == "Person");
        return failures == 0 ? 42 : 1;
    }
}
