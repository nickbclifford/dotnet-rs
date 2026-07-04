using System;
using System.Reflection;

public sealed class FieldProbe
{
    public int Number = 42;
    public string Name = "field";
}

public static class Program
{
    public static int Main()
    {
        FieldProbe probe = new FieldProbe();

        FieldInfo? number = typeof(FieldProbe).GetField(nameof(FieldProbe.Number));
        if (number == null)
        {
            return 1;
        }

        object? numberValue = number.GetValue(probe);
        if (!Equals(numberValue, 42))
        {
            return 2;
        }

        FieldInfo? name = typeof(FieldProbe).GetField(nameof(FieldProbe.Name));
        if (name == null)
        {
            return 3;
        }

        object? nameValue = name.GetValue(probe);
        if (!Equals(nameValue, "field"))
        {
            return 4;
        }

        return 42;
    }
}
