using System;
using System.Collections.Generic;
using System.Reflection;

public static class Program
{
    public static void Strings(string[] value) { }
    public static void Objects(object[] value) { }
    public static void List(IEnumerable<object> value) { }
    public static void LongRef(ref long value) { }
    static bool Reject(Action action) { try { action(); return false; } catch (ArgumentException) { return true; } }
    public static int Main()
    {
        var strings = typeof(Program).GetMethod(nameof(Strings))!;
        var objects = typeof(Program).GetMethod(nameof(Objects))!;
        var list = typeof(Program).GetMethod(nameof(List))!;
        strings.Invoke(null, new object[] { new string[0] });
        objects.Invoke(null, new object[] { new string[0] });
        list.Invoke(null, new object[] { new string[0] });
        if (!Reject(() => strings.Invoke(null, new object[] { new object[0] }))) return 1;
        if (!Reject(() => objects.Invoke(null, new object[] { new int[0] }))) return 2;
        if (!Reject(() => list.Invoke(null, new object[] { new int[0] }))) return 3;
        if (!Reject(() => typeof(Program).GetMethod(nameof(LongRef))!.Invoke(null, new object[] { 1 }))) return 4;
        try { typeof(string).GetMethod(nameof(string.ToString), Type.EmptyTypes)!.Invoke(new int[0], null); return 5; }
        catch (TargetException) { }
        return 42;
    }
}
