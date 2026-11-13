using System;
using System.Reflection;

public static class Target
{
    public static int Write(out int value)
    {
        value = 42;
        return value;
    }
}

public static class Program
{
    public static int Main()
    {
        MethodInfo? method = typeof(Target).GetMethod(nameof(Target.Write));
        if (method is null)
        {
            return 1;
        }

        object[] args = new object[] { -1 };
        object original = args[0];
        object? result = method.Invoke(null, args);

        if ((int)original != -1 || object.ReferenceEquals(original, args[0])) return 4;

        if (result is not int intResult || intResult != 42)
        {
            return 2;
        }

        if (args[0] is not int outValue || outValue != 42)
        {
            return 3;
        }

        return 42;
    }
}
