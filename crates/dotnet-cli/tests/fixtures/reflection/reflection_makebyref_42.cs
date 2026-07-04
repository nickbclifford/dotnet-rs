using System;

public sealed class ByRefProbe
{
}

public static class Program
{
    public static int Main()
    {
        Type byRef = typeof(ByRefProbe).MakeByRefType();

        if (!byRef.IsByRef)
        {
            return 1;
        }

        if (byRef.GetElementType() != typeof(ByRefProbe))
        {
            return 2;
        }

        if (typeof(ByRefProbe).MakeByRefType() != byRef)
        {
            return 3;
        }

        return 42;
    }
}
