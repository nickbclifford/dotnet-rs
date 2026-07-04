using System;
using System.Reflection;

public static class MethodProbe
{
    public static string Target() => "nongeneric";
    public static string Target<T>() => typeof(T).Name;
    public static string Target<T>(int value) => typeof(T).Name + ":" + value.ToString();
}

public static class Program
{
    public static int Main()
    {
        Type type = typeof(MethodProbe);
        const BindingFlags flags = BindingFlags.Public | BindingFlags.Static;

        MethodInfo? nongeneric = type.GetMethod("Target", 0, flags, null, Type.EmptyTypes, null);
        if (nongeneric == null || nongeneric.IsGenericMethodDefinition)
        {
            return 1;
        }

        object? nongenericResult = nongeneric.Invoke(null, Array.Empty<object>());
        if (!Equals(nongenericResult, "nongeneric"))
        {
            return 2;
        }

        MethodInfo? generic = type.GetMethod("Target", 1, flags, null, Type.EmptyTypes, null);
        if (generic == null || !generic.IsGenericMethodDefinition)
        {
            return 3;
        }

        object? genericResult = generic.MakeGenericMethod(typeof(int)).Invoke(null, Array.Empty<object>());
        if (!Equals(genericResult, "Int32"))
        {
            return 4;
        }

        MethodInfo? genericWithParameter = type.GetMethod("Target", 1, flags, null, new[] { typeof(int) }, null);
        if (genericWithParameter == null)
        {
            return 5;
        }

        object? parameterResult = genericWithParameter.MakeGenericMethod(typeof(string)).Invoke(null, new object[] { 7 });
        if (!Equals(parameterResult, "String:7"))
        {
            return 6;
        }

        MethodInfo? mismatch = type.GetMethod("Target", 1, flags, null, new[] { typeof(string) }, null);
        if (mismatch != null)
        {
            return 7;
        }

        return 42;
    }
}
