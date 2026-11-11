using System;
using System.Globalization;
using System.Threading.Tasks;

public static class Program
{
    public static int Main()
    {
        CultureInfo.CurrentUICulture = CultureInfo.InvariantCulture;
        string aggregate = new AggregateException().Message;
        string canceled = new TaskCanceledException().Message;
        return aggregate == "One or more errors occurred." && canceled == "A task was canceled." ? 42 : 1;
    }
}
