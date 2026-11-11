using System.Threading.Tasks;

public static class Program
{
    public static int Main() => Task.Run(() => 42).Result;
}
