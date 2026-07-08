using System;
using System.Linq;
using System.Reflection;

public static class Program
{
    public static int Main()
    {
        MethodInfo viaGetMethods = typeof(Queryable)
            .GetMethods()
            .Single(m => m.Name == nameof(Queryable.First)
                && m.IsGenericMethodDefinition
                && m.GetParameters().Length == 1);

        MethodInfo viaDelegate = new Func<IQueryable<object>, object>(Queryable.First)
            .Method
            .GetGenericMethodDefinition();

        bool opEq = viaGetMethods == viaDelegate;
        bool refEq = object.ReferenceEquals(viaGetMethods, viaDelegate);

        Console.WriteLine($"Queryable.First<T> == : {opEq}");
        Console.WriteLine($"Queryable.First<T> ReferenceEquals: {refEq}");

        return opEq && refEq ? 42 : 1;
    }
}
