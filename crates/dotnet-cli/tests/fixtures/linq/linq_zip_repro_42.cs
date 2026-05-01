using System.Collections.Generic;
using System.Linq;

public static class Program
{
    public static int Main()
    {
        List<int> left = new List<int> { 1, 2, 3, 4 };
        List<int> right = new List<int> { 10, 20, 30 };
        int[] expected = new[] { 11, 22, 33 };

        int index = 0;
        int sum = 0;
        foreach (int value in left.Zip(right, (a, b) => a + b))
        {
            if (index >= expected.Length) return 1;
            if (value != expected[index]) return 2;
            sum += value;
            index++;
        }

        if (index != expected.Length) return 3;
        if (sum != 66) return 4;
        return 42;
    }
}
