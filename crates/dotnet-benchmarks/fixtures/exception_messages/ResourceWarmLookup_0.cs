using System;
using System.Globalization;
using System.Threading.Tasks;

public static class Program
{
    public static void Main()
    {
        CultureInfo.CurrentUICulture = CultureInfo.InvariantCulture;
        int found = 0;
        for (int i = 0; i < 128; i++)
        {
            try
            {
                if ((i & 1) == 0) throw new AggregateException();
                throw new TaskCanceledException();
            }
            catch (Exception exception)
            {
                if (!string.IsNullOrEmpty(exception.Message)) found++;
            }
        }
        if (found != 128) throw new Exception("Missing resource messages");
    }
}
