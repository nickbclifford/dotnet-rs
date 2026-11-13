using System;
using System.Reflection;

public static class Target
{
    public static void Update(ref int value, ref object item)
    {
        value = 42;
        item = "changed";
        throw new InvalidOperationException();
    }
}

public static class Program
{
    public static int Main()
    {
        object[] args = { 1, "before" };
        object original = args[0];
        try
        {
            typeof(Target).GetMethod(nameof(Target.Update))!.Invoke(null, args);
            return 1;
        }
        catch (TargetInvocationException e) when (e.InnerException is InvalidOperationException)
        {
            if (!object.ReferenceEquals(original, args[0])) return 2;
            if ((int)args[0] != 1 || (string)args[1] != "before") return 3;
        }
        return 42;
    }
}
