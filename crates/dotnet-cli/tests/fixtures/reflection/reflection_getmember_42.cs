using System;
using System.Reflection;

public sealed class MemberProbe
{
    public int Number;

    public string Name { get; set; } = string.Empty;

    public void Touch()
    {
    }

    private int Hidden() => 7;
}

public static class Program
{
    public static int Main()
    {
        const BindingFlags flags =
            BindingFlags.Instance |
            BindingFlags.Public |
            BindingFlags.NonPublic |
            BindingFlags.DeclaredOnly;

        MemberInfo[] properties = typeof(MemberProbe).GetMember("Name", MemberTypes.Property, flags);
        if (properties.Length != 1 || properties[0].MemberType != MemberTypes.Property)
        {
            return 1;
        }

        MemberInfo[] fields = typeof(MemberProbe).GetMember("Number", MemberTypes.Field, flags);
        if (fields.Length != 1 || fields[0].MemberType != MemberTypes.Field)
        {
            return 2;
        }

        MemberInfo[] methods = typeof(MemberProbe).GetMember("Touch", MemberTypes.Method, flags);
        if (methods.Length != 1 || methods[0].MemberType != MemberTypes.Method)
        {
            return 3;
        }

        MemberInfo[] any = typeof(MemberProbe).GetMember("Hidden", flags);
        if (any.Length != 1 || any[0].MemberType != MemberTypes.Method)
        {
            return 4;
        }

        MemberInfo[] missing = typeof(MemberProbe).GetMember("Missing", MemberTypes.All, flags);
        if (missing.Length != 0)
        {
            return 5;
        }

        return 42;
    }
}
