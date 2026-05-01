using System.Collections.Generic;
using System.Linq;

public static class Program
{
    private static IEnumerable<int> Source()
    {
        yield return 5;
        yield return 7;
        yield return 9;
        yield return 11;
    }

    public static int Main()
    {
        int threshold = 6;

        int first = Source()
            .Where(value => value > threshold)
            .Select(value => value - 1)
            .First();

        int count = Source()
            .Where(value => value > threshold)
            .Select(value => value - 1)
            .Count();

        bool any = Source()
            .Where(value => value > threshold)
            .Select(value => value - 1)
            .Any(value => value == 8);

        bool all = Source()
            .Where(value => value > threshold)
            .Select(value => value - 1)
            .All(value => value % 2 == 0);

        return first == 6 && count == 3 && any && all ? 42 : 1;
    }
}
