using System;
using System.Collections.Generic;
using System.Linq.Expressions;

public static class Program
{
    private delegate int Sum4(int a, int b, int c, int d);

    private delegate void Fill3(List<int> list, int value, int count);

    private static int Fail(int code)
    {
        Console.WriteLine("FAIL " + code);
        return code;
    }

    public static int Main()
    {
        // A user-declared delegate type, built through the non-generic Expression.Lambda(Type, ...).
        ParameterExpression a = Expression.Parameter(typeof(int), "a");
        ParameterExpression b = Expression.Parameter(typeof(int), "b");
        ParameterExpression c = Expression.Parameter(typeof(int), "c");
        ParameterExpression d = Expression.Parameter(typeof(int), "d");
        Expression body = Expression.Add(Expression.Add(a, b), Expression.Add(c, d));
        LambdaExpression lambda = Expression.Lambda(typeof(Sum4), body, a, b, c, d);
        Sum4 sum = (Sum4)lambda.Compile();
        if (sum(10, 10, 10, 12) != 42) return Fail(1);

        // A user-declared void delegate with reference-type and value-type parameters.
        ParameterExpression list = Expression.Parameter(typeof(List<int>), "list");
        ParameterExpression value = Expression.Parameter(typeof(int), "value");
        ParameterExpression count = Expression.Parameter(typeof(int), "count");
        ParameterExpression i = Expression.Variable(typeof(int), "i");
        LabelTarget done = Expression.Label();
        Expression loop = Expression.Block(
            new[] { i },
            Expression.Assign(i, Expression.Constant(0)),
            Expression.Loop(
                Expression.IfThenElse(
                    Expression.LessThan(i, count),
                    Expression.Block(
                        Expression.Call(list, typeof(List<int>).GetMethod("Add")!, value),
                        Expression.PreIncrementAssign(i)),
                    Expression.Break(done)),
                done));
        Fill3 fill = (Fill3)Expression.Lambda(typeof(Fill3), loop, list, value, count).Compile();
        List<int> target = new List<int>();
        fill(target, 14, 3);
        if (target.Count != 3 || target[0] + target[1] + target[2] != 42) return Fail(2);

        // Delegate type inferred from the parameter types by Expression.GetDelegateType.
        Type inferred = Expression.GetDelegateType(typeof(int), typeof(int), typeof(int), typeof(int));
        if (inferred != typeof(Func<int, int, int, int>)) return Fail(3);

        // A typed Expression<TDelegate> invoked from inside another lambda.
        Expression<Func<int, int, int, int>> inner = Expression.Lambda<Func<int, int, int, int>>(
            Expression.Multiply(Expression.Add(a, b), c), a, b, c);
        ParameterExpression x = Expression.Parameter(typeof(int), "x");
        Expression<Func<int, int>> outer = Expression.Lambda<Func<int, int>>(
            Expression.Invoke(inner, x, Expression.Constant(5), Expression.Constant(2)), x);
        if (outer.Compile()(16) != 42) return Fail(4);

        Console.WriteLine("ok");
        return 42;
    }
}
