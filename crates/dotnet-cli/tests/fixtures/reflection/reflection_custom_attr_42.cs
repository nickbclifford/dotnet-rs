using System;

[AttributeUsage(AttributeTargets.Class)]
public sealed class IntValueAttribute : Attribute
{
    public int Value { get; }

    public IntValueAttribute(int value)
    {
        Value = value;
    }
}

[IntValue(42)]
public sealed class AnnotatedType
{
}

public static class Program
{
    public static int Main()
    {
        object[] attrs = typeof(AnnotatedType).GetCustomAttributes(inherit: false);
        foreach (object attr in attrs)
        {
            if (attr is IntValueAttribute intValue)
            {
                return intValue.Value;
            }
        }

        return 1;
    }
}
