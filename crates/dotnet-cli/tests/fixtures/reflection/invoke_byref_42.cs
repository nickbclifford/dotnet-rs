using System;
using System.Reflection;

public static class Target
{
    public static int SetFlag(ref bool flag)
    {
        if (flag)
        {
            return -1;
        }

        flag = true;
        return 42;
    }
}

public static class Program
{
    public static int Main()
    {
        MethodInfo? method = typeof(Target).GetMethod(nameof(Target.SetFlag));
        if (method is null)
        {
            return 1;
        }

        object[] args = new object[] { false };
        object? result = method.Invoke(null, args);

        if (result is not int intResult || intResult != 42)
        {
            return 2;
        }

        if (args[0] is not bool updated || !updated)
        {
            return 3;
        }

        return 42;
    }
}
