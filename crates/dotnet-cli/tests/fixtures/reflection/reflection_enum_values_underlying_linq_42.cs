using System;
using System.Linq;

public enum State
{
    Detached = 0,
    Unchanged = 1,
    Deleted = 2,
    Modified = 3,
    Added = 4
}

public static class Program
{
    public static int Main()
    {
        int max = Enum.GetValuesAsUnderlyingType<State>().Cast<int>().Max();
        return max == 4 ? 42 : 1;
    }
}
