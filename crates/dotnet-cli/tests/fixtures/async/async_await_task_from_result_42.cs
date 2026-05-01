using System.Threading.Tasks;

public static class Program
{
    private static async Task<int> ComputeAsync()
    {
        int value = await Task.FromResult(40);
        return value + 2;
    }

    public static int Main()
    {
        return ComputeAsync().Result;
    }
}
