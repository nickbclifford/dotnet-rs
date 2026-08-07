using System;

public interface IStaticConstrained<TSelf>
    where TSelf : IStaticConstrained<TSelf>
{
    static abstract int Increment<TMarker>(int value);

    static virtual int DefaultContribution<TMarker>()
        => typeof(TMarker) == typeof(string) ? 1 : 2;
}

public readonly struct StaticConstrained : IStaticConstrained<StaticConstrained>
{
    public static int Increment<TMarker>(int value)
        => typeof(TMarker) == typeof(string) ? value + 1 : value + 2;
}

public class Program
{
    private static int Invoke<TSelf, TMarker>(int value)
        where TSelf : IStaticConstrained<TSelf>
    {
        return TSelf.Increment<TMarker>(value) + TSelf.DefaultContribution<TMarker>();
    }

    public static int Main()
    {
        var first = Invoke<StaticConstrained, string>(40);
        var second = Invoke<StaticConstrained, string>(40);
        return first == 42 && second == 42 ? 42 : 1;
    }
}
