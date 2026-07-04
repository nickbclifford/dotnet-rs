using System;
using System.Runtime.CompilerServices;

public sealed class ThrowsIfConstructed
{
    public static int CtorCalls;
    public int Value = 7;

    public ThrowsIfConstructed()
    {
        CtorCalls++;
        Value = 999;
    }
}

public struct Pair
{
    public int Left;
    public object? Right;
}

public static class Program
{
    public static int Main()
    {
        var boxedPair = RuntimeHelpers.GetUninitializedObject(typeof(Pair));
        if (boxedPair is not Pair pair)
        {
            return 1;
        }

        if (pair.Left != 0 || pair.Right != null)
        {
            return 2;
        }

        var boxedClass = RuntimeHelpers.GetUninitializedObject(typeof(ThrowsIfConstructed));
        var instance = (ThrowsIfConstructed)boxedClass;

        if (ThrowsIfConstructed.CtorCalls != 0)
        {
            return 3;
        }

        if (instance.Value != 0)
        {
            return 4;
        }

        return 42;
    }
}
