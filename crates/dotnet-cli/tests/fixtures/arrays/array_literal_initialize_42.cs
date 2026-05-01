using System;

public static class Program
{
    public static int Main()
    {
        int[] values = new[] { 1, 2, 3, 4, 5, 6, 7, 8 };
        int sum = 0;

        for (int i = 0; i < values.Length; i++)
        {
            sum += values[i];
        }

        return sum == 36 ? 42 : 1;
    }
}
