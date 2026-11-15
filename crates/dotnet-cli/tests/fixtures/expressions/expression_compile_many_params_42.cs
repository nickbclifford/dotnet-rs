using System;
using System.Linq.Expressions;

public static class Program
{
    private struct Pair
    {
        public int A { get; set; }
        public long B { get; set; }
    }

    private static int s_sink;

    private static int Fail(int code)
    {
        Console.WriteLine("FAIL " + code);
        return code;
    }

    public static int Main()
    {
        // Three parameters: wider than the two-parameter C# thunks.
        Expression<Func<int, int, int, int>> three = (a, b, c) => a + b + c;
        if (three.Compile()(10, 20, 12) != 42) return Fail(1);

        // Eight parameters with mixed value and reference types.
        Expression<Func<int, long, double, string, bool, char, byte, object, string>> eight =
            (i, l, d, s, f, c, b, o) => s + i + l + d + f + c + b + o;
        string joined = eight.Compile()(1, 2L, 3.5, "x", true, 'c', (byte)7, "o");
        if (joined != "x123.5Truec7o") return Fail(2);

        // Value-type parameters and a value-type result.
        Expression<Func<Pair, Pair, Pair, Pair>> pairs = (p, q, r) =>
            MakePair(p.A + q.A + r.A, p.B + q.B + r.B);
        Pair sum = pairs.Compile()(MakePair(1, 10), MakePair(2, 20), MakePair(3, 30));
        if (sum.A != 6 || sum.B != 60) return Fail(3);

        // A void-returning delegate with four parameters.
        Expression<Action<int, int, int, int>> action = (a, b, c, d) => Record(a, b, c, d);
        action.Compile()(1, 2, 3, 36);
        if (s_sink != 42) return Fail(4);

        // The widest Func type: sixteen parameters.
        Expression<Func<int, int, int, int, int, int, int, int, int, int, int, int, int, int, int, int, int>> wide =
            (a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p) => a + b + c + d + e + f + g + h + i + j + k + l + m + n + o + p;
        if (wide.Compile()(1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 27) != 42) return Fail(6);

        // A wrongly typed argument is rejected before the thunk runs.
        Delegate untyped = three.Compile();
        try
        {
            untyped.DynamicInvoke(1, 2, "three");
            return Fail(7);
        }
        catch (ArgumentException)
        {
        }

        Console.WriteLine("ok");
        return 42;
    }

    private static Pair MakePair(int a, long b)
    {
        Pair pair = default;
        pair.A = a;
        pair.B = b;
        return pair;
    }

    private static void Record(int a, int b, int c, int d)
    {
        s_sink = a + b + c + d;
    }
}
