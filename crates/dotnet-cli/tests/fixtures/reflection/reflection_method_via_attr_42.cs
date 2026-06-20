using System;
using System.Reflection;

[AttributeUsage(AttributeTargets.Method)]
public sealed class TargetAttribute : Attribute
{
}

public sealed class CandidateMethods
{
    public int IgnoreA() => 1;

    [Target]
    public int Selected() => 42;

    public int IgnoreB() => 2;
}

public static class Program
{
    public static int Main()
    {
        Type type = typeof(CandidateMethods);
        MethodInfo? targetMethod = null;

        foreach (MethodInfo method in type.GetMethods(BindingFlags.Instance | BindingFlags.Public | BindingFlags.DeclaredOnly))
        {
            object[] attributes = method.GetCustomAttributes(inherit: false);
            foreach (object attribute in attributes)
            {
                if (attribute is TargetAttribute)
                {
                    targetMethod = method;
                    break;
                }
            }

            if (targetMethod != null)
            {
                break;
            }
        }

        if (targetMethod == null)
        {
            return 1;
        }

        CandidateMethods instance = new CandidateMethods();
        object? result = targetMethod.Invoke(instance, Array.Empty<object>());

        if (result is int value)
        {
            return value;
        }

        return 2;
    }
}
