using System.Threading.Tasks;

public static class Program
{
    private static readonly TaskCompletionSource<int> Completion = new();

    private static async Task<int> ComputeAsync()
    {
        int value = await Completion.Task;
        return value + 2;
    }

    public static int Main()
    {
        Task<int> task = ComputeAsync();
        if (task.IsCompleted)
        {
            return 1;
        }

        Completion.SetResult(40);

        if (!task.IsCompleted)
        {
            return 2;
        }

        return task.Result;
    }
}
