using System.Threading;
using System.Threading.Tasks;

public static class Program
{
    private static readonly AsyncLocal<string> Ambient = new();
    private static TaskCompletionSource<bool> Release = null!;

    private static async Task<int> ReadAfterYield()
    {
        await Task.Yield();
        bool captured = Ambient.Value == "captured";
        await Release.Task;
        return captured ? 42 : 1;
    }

    public static async Task<int> Main()
    {
        Ambient.Value = "captured";
        Release = new TaskCompletionSource<bool>();
        Task<int> result = ReadAfterYield();
        bool suspended = !result.IsCompleted;
        Ambient.Value = "parent";
        Release.SetResult(true);

        int observed = await result;
        return suspended && observed == 42 && Ambient.Value == "parent" ? 42 : 1;
    }
}
