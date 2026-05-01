using System.Threading.Tasks;

public static class Program
{
    public static int Main()
    {
        ValueTask<int> valueTask = new(42);
        return valueTask.Result;
    }
}
