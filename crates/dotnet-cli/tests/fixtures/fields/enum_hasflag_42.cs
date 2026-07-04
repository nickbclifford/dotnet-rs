using System;

[Flags]
enum SmallFlags : sbyte
{
    None = 0,
    One = 1,
    Two = 2,
    All = -1
}

[Flags]
enum IntFlags
{
    None = 0,
    One = 1,
    Two = 2,
    Four = 4
}

enum OtherFlags
{
    One = 1
}

public class Program
{
    public static int Main()
    {
        IntFlags value = IntFlags.One | IntFlags.Four;

        if (!value.HasFlag(IntFlags.One))
            return 1;
        if (value.HasFlag(IntFlags.Two))
            return 2;
        if (!value.HasFlag(IntFlags.None))
            return 3;

        SmallFlags all = SmallFlags.All;
        if (!all.HasFlag(SmallFlags.Two))
            return 4;

        Enum other = OtherFlags.One;
        try
        {
            value.HasFlag(other);
            return 5;
        }
        catch (ArgumentException)
        {
        }

        return 42;
    }
}
