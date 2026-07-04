using System;
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

        if (method.Name != "GetHashCode")
        {
            return 2;
        }

        if (method.GetParameters().Length != 0)
        {
            return 3;
        }

        return 42;
    }
}
