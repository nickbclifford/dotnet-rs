using System;
using System.Reflection;
using System.Runtime.CompilerServices;

public class ByNew { public static int V; public int I; static ByNew() { V = 1; } public ByNew(int a) { I = a + V; } }
public class ByCall { public static int V; static ByCall() { V = 2; } public static int Add(int a, int b) { return a + b + V; } }
public class ByDelegate { public static int V; public int I = 3; static ByDelegate() { V = 4; } public static int Add(int a) { return a + V; } public int Inst(int a) { return a + I + V; } }
public class ByMulticast { public static int Count; static ByMulticast() { Count = 100; } public static void Bump() { Count++; } }
public class ByReflection { public static int V; static ByReflection() { V = 6; } public static int Get(int a) { return a + V; } }
public struct ByValue { public int X; public static int V; static ByValue() { V = 7; } public int Get() { return X + V; } }

public class Program
{
    public static int Main()
    {
        if (new ByNew(10).I != 11) return 1;
        if (ByCall.Add(1, 2) != 5) return 2;
        Func<int, int> f = ByDelegate.Add;
        if (f(20) != 24) return 3;
        Func<int, int> g = new ByDelegate().Inst;
        if (g(1) != 8) return 4;
        Action a = ByMulticast.Bump;
        a += ByMulticast.Bump;
        a();
        if (ByMulticast.Count != 102) return 5;
        if ((int)typeof(ByReflection).GetMethod("Get").Invoke(null, new object[] { 5 }) != 11) return 6;
        ByValue v = default; v.X = 1;
        if (v.Get() != 8) return 7;
        return 42;
    }
}
