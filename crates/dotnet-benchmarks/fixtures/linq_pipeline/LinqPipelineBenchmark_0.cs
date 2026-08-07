using System.Collections.Generic;
using System.Linq;

public sealed class Purchase
{
    public int CustomerId { get; set; }
    public int Amount { get; set; }
    public bool IsActive { get; set; }
}

public static class Program
{
    public static int Main()
    {
        var purchases = new List<Purchase>(720);
        for (var i = 0; i < 720; i++)
        {
            purchases.Add(new Purchase
            {
                CustomerId = i % 37,
                Amount = ((i * 17) % 101) + 1,
                IsActive = i % 5 != 0,
            });
        }

        var checksum = 0;
        for (var iteration = 0; iteration < 10; iteration++)
        {
            var groups = purchases
                .Where(purchase => purchase.IsActive && purchase.Amount >= 20)
                .Select(purchase => purchase.Amount + purchase.CustomerId)
                .GroupBy(value => value % 11)
                .OrderBy(group => group.Key);

            foreach (var group in groups)
            {
                checksum += group.Key;
                checksum += group.Sum();
                checksum += group.Count();
            }
        }

        return checksum > 0 ? 0 : 1;
    }
}
