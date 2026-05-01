using System;
using System.Threading.Tasks;

public static class Program
{
    private static async Task<int> ComputeStepAsync(int value)
    {
        int observed = await Task.FromResult(value);
        if ((observed % 3) == 1)
        {
            throw new InvalidOperationException("step");
        }

        return observed * 2;
    }

    private static async Task<int> RunAsync()
    {
        int total = 0;

        for (int i = 0; i < 6; i++)
        {
            try
            {
                total += await ComputeStepAsync(i);
            }
            catch (InvalidOperationException ex)
            {
                if (ex.Message != "step")
                {
                    return -1;
                }

                total += 5;
            }
            finally
            {
                total += 2;
            }
        }

        return total;
    }

    public static int Main()
    {
        return RunAsync().Result;
    }
}
