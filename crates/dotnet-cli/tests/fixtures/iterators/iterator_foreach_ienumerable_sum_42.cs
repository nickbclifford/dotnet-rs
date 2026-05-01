using System.Collections.Generic;

public static class Program
{
    public static IEnumerable<int> Source()
    {
        yield return 30;
        yield return 12;
    }

    public static int Sum(IEnumerable<int> values)
    {
        int sum = 0;
        foreach (int value in values)
        {
            sum += value;
        }

        return sum;
    }

    public static int Main()
    {
        IEnumerable<int> numbers = Source();
        return Sum(numbers) == 42 ? 42 : 1;
    }
}
