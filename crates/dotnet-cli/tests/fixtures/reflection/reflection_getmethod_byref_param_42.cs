using System;
using System.Reflection;

public static class ByRefMethodProbe
{
    public static void Touch(ref int value)
    {
        value += 1;
    }
}

public static class Program
{
    public static int Main()
    {
        MethodInfo? method = typeof(ByRefMethodProbe).GetMethod(
            "Touch",
            new[] { typeof(int).MakeByRefType() }
        );

        if (method == null)
        {
            return 1;
        }

        ParameterInfo[] parameters = method.GetParameters();
        if (parameters.Length != 1)
        {
            return 2;
        }

        Type parameterType = parameters[0].ParameterType;
        if (!parameterType.IsByRef || parameterType.GetElementType() != typeof(int))
        {
            return 3;
        }

        return 42;
    }
}
