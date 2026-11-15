using System;
using System.Collections.Generic;

// Type.IsGenericParameter, and Type.ContainsGenericParameters, which is built on it.
public static class Program
{
    static int failures;
    static void Check(int id, bool ok)
    {
        if (!ok) { failures++; Console.WriteLine("FAIL " + id); }
    }
    static void InMethod<U>()
    {
        Check(7, !typeof(U).IsGenericParameter);
        Check(8, !typeof(List<U>).ContainsGenericParameters);
    }
    public static int Main()
    {
        Check(1, typeof(Dictionary<,>).GetGenericArguments()[1].IsGenericParameter);
        Check(2, !typeof(int).IsGenericParameter);
        Check(3, !typeof(List<int>).IsGenericParameter);
        Check(4, typeof(List<>).ContainsGenericParameters);
        Check(5, !typeof(List<int>).ContainsGenericParameters);
        Check(6, typeof(List<>).MakeArrayType().ContainsGenericParameters);
        InMethod<int>();
        Console.WriteLine(failures == 0 ? "ok" : "failures: " + failures);
        return failures == 0 ? 42 : 1;
    }
}
