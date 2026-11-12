using System;

public class A
{
    public static int X;
    public int I;
    public A() { I = X; }
    public static int Get() { return X; }
    static A() { X = new B().I + 1; }
}

public class B
{
    public static int Y;
    public int I;
    public B() { I = Y; }
    static B() { Y = A.Get() + 1; Func<int> f = A.Get; Y += f(); }
}

public class Program
{
    public static int Main()
    {
        // Entering A starts B, whose A.Get/delegate calls recurse into A on the same thread.
        int x = A.Get();
        return x == 2 && B.Y == 1 ? 42 : x * 10 + B.Y;
    }
}
