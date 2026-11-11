using System;
using System.Threading.Tasks;

public static class Program
{
    public static async Task<int> Main()
    {
        long started = Environment.TickCount64;
        await Task.Delay(50);
        return Environment.TickCount64 - started >= 50 ? 42 : 1;
    }
}
