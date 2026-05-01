using System.Threading.Tasks;

public static class Program
{
    private static async ValueTask<int> ComputeAsync()
    {
        int value = await new ValueTask<int>(40);
        return value + 2;
    }

    public static int Main()
    {
        return ComputeAsync().Result;
    }
}
