using System;

class GenericStaticHolder<TKey, TValue>
{
    public static readonly object Empty = new object();
}

static class GenericFactory
{
    public static object Create<TKey, TValue>()
    {
        return GenericStaticHolder<TKey, TValue>.Empty;
    }
}

class Outer<TOuter>
{
    public object CallFactory()
    {
        // Call from a generic type context so the caller frame carries type-generics.
        return GenericFactory.Create<int, string>();
    }
}

public class Program
{
    public static int Main()
    {
        var outer = new Outer<long>();
        var first = outer.CallFactory();
        var second = outer.CallFactory();
        var swapped = GenericFactory.Create<string, int>();

        if (first == null || second == null || swapped == null)
            return 1;
        if (!object.ReferenceEquals(first, second))
            return 2;
        if (object.ReferenceEquals(first, swapped))
            return 3;

        return 42;
    }
}
