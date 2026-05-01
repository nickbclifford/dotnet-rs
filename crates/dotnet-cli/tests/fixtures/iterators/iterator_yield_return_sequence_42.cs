using System.Collections.Generic;

public static class Program
{
    public static IEnumerable<int> Numbers()
    {
        yield return 10;
        yield return 20;
        yield return 12;
    }

    public static int Main()
    {
        int count = 0;
        int sum = 0;

        foreach (int value in Numbers())
        {
            count++;
            sum += value;
        }

        return count == 3 && sum == 42 ? 42 : 1;
    }
}
