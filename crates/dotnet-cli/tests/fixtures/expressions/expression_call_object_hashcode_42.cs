using System;
using System.Linq.Expressions;
using System.Reflection;

public static class Program
{
    public static int Main()
    {
        MethodInfo? method = typeof(object).GetRuntimeMethod("GetHashCode", Type.EmptyTypes);
        if (method == null)
        {
            return 1;
        }

        ParameterExpression parameter = Expression.Parameter(typeof(int), "v");
        MethodCallExpression call = Expression.Call(parameter, method);

        if (call.Type != typeof(int))
        {
            return 2;
        }

        return 42;
    }
}
