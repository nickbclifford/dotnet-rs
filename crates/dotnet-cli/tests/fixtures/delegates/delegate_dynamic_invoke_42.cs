using System;

public class Program
{
    static int Add(int a, int b) => a + b;
    int offset = 10;
    string Concat(string s, int n) => s + (n + offset);
    static void Nothing() { }
    static int Throws() { throw new InvalidOperationException("x"); }

    delegate int AddDel(int a, int b);

    public static int Main()
    {
        Delegate d = new AddDel(Add);
        object r = d.DynamicInvoke(20, 22);
        if (!(r is int i) || i != 42) return 1;

        Func<string, int, string> f = new Program().Concat;
        if ((string)f.DynamicInvoke("a", 5) != "a15") return 2;

        Action a = Nothing;
        if (a.DynamicInvoke() != null) return 3;

        try
        {
            ((Func<int>)Throws).DynamicInvoke();
            return 4;
        }
        catch (System.Reflection.TargetInvocationException e)
        {
            if (!(e.InnerException is InvalidOperationException)) return 5;
        }
        return 42;
    }
}
