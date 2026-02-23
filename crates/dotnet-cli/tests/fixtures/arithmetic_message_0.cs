using System;

class Program
{
    static int Main()
    {
        int result = 0;
        
        // Test DivideByZeroException
        try
        {
            int a = 10;
            int b = 0;
            int c = a / b;
            return 1;
        }
        catch (DivideByZeroException ex)
        {
            if (ex.Message != "Attempted to divide by zero.")
                return 2;
            result++;
        }

        // Test OverflowException (checked)
        try
        {
            checked
            {
                int a = int.MaxValue;
                int b = 1;
                int c = a + b;
            }
            return 3;
        }
        catch (OverflowException ex)
        {
            if (ex.Message != "Arithmetic operation resulted in an overflow.")
                return 4;
            result++;
        }

        if (result == 2)
            return 0; // Success
        return 5;
    }
}
