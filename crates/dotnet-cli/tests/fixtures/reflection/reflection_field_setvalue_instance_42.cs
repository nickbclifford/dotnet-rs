using System;
using System.Reflection;

public sealed class FieldSetProbe
{
    public int Number;
    public string? Name;
}

public static class Program
{
    public static int Main()
    {
        FieldSetProbe probe = new FieldSetProbe();

        FieldInfo? number = typeof(FieldSetProbe).GetField(nameof(FieldSetProbe.Number));
        if (number == null)
        {
            return 1;
        }

        number.SetValue(probe, 42, BindingFlags.Default, binder: null, culture: null);
        if (probe.Number != 42)
        {
            return 2;
        }

        FieldInfo? name = typeof(FieldSetProbe).GetField(nameof(FieldSetProbe.Name));
        if (name == null)
        {
            return 3;
        }

        name.SetValue(probe, "updated", BindingFlags.Default, binder: null, culture: null);
        if (probe.Name != "updated")
        {
            return 4;
        }

        return 42;
    }
}
