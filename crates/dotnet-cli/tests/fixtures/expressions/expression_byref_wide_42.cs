using System;
using System.Linq.Expressions;

public static class Program
{
    private delegate int RefAction(ref int value, int add, int extra);
    private delegate int Wide(int a, int b, int c, int d, int e, int f, int g, int h, int i, int j, int k, int l, int m, int n, int o, int p, int q);

    public static int Main()
    {
        var value = Expression.Parameter(typeof(int).MakeByRefType(), "value");
        var add = Expression.Parameter(typeof(int), "add");
        var extra = Expression.Parameter(typeof(int), "extra");
        var body = Expression.AddAssign(value, Expression.Add(add, extra));
        var update = Expression.Lambda<RefAction>(body, value, add, extra).Compile(preferInterpretation: true);
        int result = 10;
        if (update(ref result, 20, 12) != 42 || result != 42) return 1;

        var throwingBody = Expression.Block(
            Expression.Assign(value, Expression.Constant(42)),
            Expression.Throw(Expression.New(typeof(InvalidOperationException)), typeof(int)));
        var throwing = Expression.Lambda<RefAction>(throwingBody, value, add, extra).Compile(preferInterpretation: true);
        result = 1;
        try { throwing(ref result, 0, 0); return 3; }
        catch (InvalidOperationException) { }
        if (result != 42) return 4;

        var args = new ParameterExpression[17];
        for (int i = 0; i < args.Length; i++) args[i] = Expression.Parameter(typeof(int), "p" + i);
        var wide = Expression.Lambda<Wide>(Expression.Add(args[0], args[16]), args).Compile(preferInterpretation: true);
        if (wide(20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 22) != 42) return 2;
        Console.WriteLine("ok");
        return 42;
    }
}
