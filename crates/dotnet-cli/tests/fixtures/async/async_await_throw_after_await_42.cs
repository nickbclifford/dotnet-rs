using System;
using System.Threading.Tasks;

public static class Program
{
    private static async Task<int> ComputeAsync()
    {
        int value = await Task.FromResult(40);
        if (value != 40)
        {
            return -1;
        }

        throw new InvalidOperationException("boom");
    }

    public static int Main()
    {
        try
        {
            _ = ComputeAsync().Result;
            return 1;
        }
        catch (InvalidOperationException ex)
        {
            return ex.Message == "boom" ? 42 : 2;
        }
    }
}
