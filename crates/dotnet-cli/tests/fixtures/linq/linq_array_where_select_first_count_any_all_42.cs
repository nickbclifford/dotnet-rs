using System.Linq;

public static class Program
{
    public static int Main()
    {
        int[] values = new[] { 1, 2, 3, 4, 5, 6, 7, 8 };

        int first = values
            .Where(value => value % 2 == 0)
            .Select(value => value * 3)
            .First();

        int count = values
            .Where(value => value % 2 == 0)
            .Select(value => value * 3)
            .Count();

        bool any = values
            .Where(value => value % 2 == 0)
            .Select(value => value * 3)
            .Any(value => value == 18);

        bool all = values
            .Where(value => value % 2 == 0)
            .Select(value => value * 3)
            .All(value => value % 6 == 0);

        return first == 6 && count == 4 && any && all ? 42 : 1;
    }
}
