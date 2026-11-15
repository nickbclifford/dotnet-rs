using System;

// Open-instance delegates (Func<T, R> over an instance method of T) pass their first Invoke argument
// as `this`. The VM used to push the delegate's null target as well, leaving a stray value that
// shifted the caller's evaluation stack: "openD|" + d lost its prefix.
public class Q
{
    public double Score { get; set; } = 2.5;
    public int Id { get; set; } = 7;
    public double Scale(double by) => Score * by;
}
public struct V
{
    public int X;
    public int Get() => X;
}
public delegate int ByRefGetter(ref V v);

public static class Program
{
    static int failures;
    static void Check(int id, bool ok)
    {
        if (!ok) { failures++; Console.WriteLine("FAIL " + id); }
    }
    static string Tag(string a, double b) => a + "|" + b;

    public static int Main()
    {
        var q = new Q();
        var getScore = typeof(Q).GetProperty("Score")!.GetMethod!.CreateDelegate<Func<Q, double>>();
        var getId = typeof(Q).GetProperty("Id")!.GetMethod!.CreateDelegate<Func<Q, int>>();
        var setScore = typeof(Q).GetProperty("Score")!.SetMethod!.CreateDelegate<Action<Q, double>>();
        var scale = typeof(Q).GetMethod("Scale")!.CreateDelegate<Func<Q, double, double>>();

        // The values below each invoke must survive it.
        Check(1, Tag("openD", getScore(q)) == "openD|2.5");
        Check(2, ("openI|" + getId(q)) == "openI|7");
        setScore(q, 4.0);
        Check(3, q.Score == 4.0);
        Check(4, Tag("args", scale(q, 0.5)) == "args|2");
        int before = 11;
        double sum = before + getScore(q) + getId(q);
        Check(5, sum == 22.0);

        // Open instance over a value type takes `ref V`.
        var getX = (ByRefGetter)typeof(V).GetMethod("Get")!.CreateDelegate(typeof(ByRefGetter));
        var v = new V { X = 9 };
        Check(6, ("x" + getX(ref v)) == "x9");

        Console.WriteLine(failures == 0 ? "ok" : "failures: " + failures);
        return failures == 0 ? 42 : 1;
    }
}
