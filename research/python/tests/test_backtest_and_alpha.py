import unittest
from decimal import Decimal

from quantlab.backtest import BacktestConfig, EventDrivenBacktester, OrderIntent, StrategyContext
from quantlab.market import BookLevel, MarketEvent, raw_hash
from quantlab.alpha_search import AlphaCosts, AlphaConfig, Bar, search_alpha


class OneShot:
    strategy_id = "test.oneshot"
    def __init__(self, side="buy", price=Decimal("101"), quantity=3):
        self.done = False; self.side = side; self.price = price; self.quantity = quantity
    def on_event(self, event, context):
        if self.done: return None
        self.done = True
        return OrderIntent("test:1", event.instrument_id, self.side, self.quantity, self.price)


def event(seq, *, kind="quote", bid=Decimal("100"), ask=Decimal("100.1"),
          bid_qty=10, ask_qty=10, last=None, qty=None, aggressor=None, recv=None):
    receive = recv or (1_000_000_000 + seq * 100_000_000)
    bids = (BookLevel(bid, bid_qty),) if bid is not None and bid_qty else ()
    asks = (BookLevel(ask, ask_qty),) if ask is not None and ask_qty else ()
    payload = {"seq": seq, "kind": kind, "bid": None if bid is None else str(bid),
               "ask": None if ask is None else str(ask), "bid_qty": bid_qty,
               "ask_qty": ask_qty, "last": None if last is None else str(last)}
    return MarketEvent(1, f"e:{seq}", "synthetic", "SYN:X", "in.x", "NSE", kind,
        receive, seq * 100_000_000, seq, None, None, None, None,
        last, qty, None, aggressor, bids, asks, raw_hash(payload), ("synthetic_fixture",))


class BacktestTests(unittest.TestCase):
    def test_marketable_limit_partial_fills_are_replay_deterministic(self):
        # The order activates against the initial displayed size, then can
        # consume only newly displayed increments, not repeated snapshots.
        events = [event(0), event(1, ask_qty=10), event(2, ask_qty=11),
                  event(3, ask_qty=12), event(4, ask_qty=13)]
        cfg = BacktestConfig(starting_cash=Decimal("10000"), order_latency_ns=1,
                             fee_bps=Decimal("1"), max_participation=Decimal("0.1"),
                             impact_coefficient_bps=Decimal("0.5"))
        a = EventDrivenBacktester(cfg).run(events, OneShot(price=Decimal("101"), quantity=3))
        b = EventDrivenBacktester(cfg).run(events, OneShot(price=Decimal("101"), quantity=3))
        self.assertEqual(a.fills, b.fills)
        self.assertEqual(sum(x.quantity for x in a.fills), 3)
        self.assertGreaterEqual(len(a.fills), 2)
        self.assertEqual(a.ending_position, 3)
        self.assertGreater(a.fees, 0)

    def test_static_mid_market_cost_decomposition_separates_spread_and_impact(self):
        events = [event(0, bid=Decimal("100"), ask=Decimal("100.1")),
                  event(1, bid=Decimal("100"), ask=Decimal("100.1"))]
        config = BacktestConfig(starting_cash=Decimal("10000"), order_latency_ns=1,
            fee_bps=Decimal("0"), max_participation=Decimal("1"),
            impact_coefficient_bps=Decimal("0"))
        result = EventDrivenBacktester(config).run(events, OneShot(price=Decimal("101"), quantity=1))
        self.assertEqual(result.net_pnl, Decimal("-0.05"))
        self.assertEqual(result.spread_cost, Decimal("0.05"))
        self.assertEqual(result.impact_cost, Decimal("0"))
        self.assertEqual(result.gross_pnl, Decimal("0.00"))

    def test_unchanged_book_snapshot_does_not_recreate_taker_liquidity(self):
        events = [event(0), event(1), event(2), event(3), event(4)]
        cfg = BacktestConfig(starting_cash=Decimal("10000"), order_latency_ns=1,
                             fee_bps=Decimal("0"), max_participation=Decimal("1"))
        result = EventDrivenBacktester(cfg).run(events, OneShot(price=Decimal("101"), quantity=20))
        # Only the displayed 10 shares at activation can be consumed once.
        self.assertEqual(sum(fill.quantity for fill in result.fills), 10)
        self.assertEqual(result.unfilled_quantity, 10)

    def test_mixed_provider_replay_requires_explicit_provider_selection(self):
        from dataclasses import replace
        one = event(0)
        two = replace(one, event_id="f:0", provider="fyers", provider_instrument_id="NSE:X")
        with self.assertRaisesRegex(ValueError, "mixed-provider"):
            EventDrivenBacktester(BacktestConfig()).run([one, two], OneShot())

    def test_passive_queue_consumes_prints_before_simulated_order(self):
        events = [
            event(0, bid=Decimal("99.9"), ask=Decimal("100.1")),
            event(1, bid=Decimal("99.9"), ask=Decimal("100.1")),
            event(2, kind="trade", bid=None, ask=None, last=Decimal("99.9"), qty=15, aggressor="sell"),
            event(3, kind="trade", bid=None, ask=None, last=Decimal("99.9"), qty=20, aggressor="sell"),
        ]
        strategy = OneShot(price=Decimal("99.9"), quantity=3)
        cfg = BacktestConfig(starting_cash=Decimal("10000"), order_latency_ns=1,
                             fee_bps=Decimal("0"), max_participation=Decimal("0.1"),
                             queue_model="conservative_l2")
        result = EventDrivenBacktester(cfg).run(events, strategy)
        self.assertEqual(sum(x.quantity for x in result.fills), 3)
        self.assertTrue(all(x.liquidity == "maker" for x in result.fills))


    def test_passive_touch_with_unknown_aggressor_is_not_a_fill(self):
        events = [
            event(0, bid=Decimal("99.9"), ask=Decimal("100.1")),
            event(1, bid=Decimal("99.9"), ask=Decimal("100.1")),
            event(2, kind="trade", bid=None, ask=None, last=Decimal("99.9"), qty=100, aggressor=None),
        ]
        cfg = BacktestConfig(starting_cash=Decimal("10000"), order_latency_ns=1,
                             fee_bps=Decimal("0"), max_participation=Decimal("1"),
                             queue_model="conservative_l2")
        result = EventDrivenBacktester(cfg).run(events, OneShot(price=Decimal("99.9"), quantity=2))
        self.assertEqual(len(result.fills), 0)
        self.assertEqual(result.unfilled_quantity, 2)

    def test_cancel_latency_is_part_of_tif_expiry(self):
        # The order stays exposed through cancel latency; no fill after cancel ACK.
        events = [event(0), event(1), event(2), event(3), event(4)]
        strategy = OneShot(price=Decimal("90"), quantity=1)
        cfg = BacktestConfig(starting_cash=Decimal("1000"), order_latency_ns=1,
                             cancel_latency_ns=10_000_000_000, fee_bps=Decimal("0"),
                             max_participation=Decimal("0.1"))
        result = EventDrivenBacktester(cfg).run(events, strategy)
        self.assertEqual(len(result.fills), 0)
        self.assertEqual(result.unfilled_quantity, 1)


    def test_streaming_ema_strategy_exits_full_position(self):
        from quantlab.strategies import DeterministicEmaCross
        strategy = DeterministicEmaCross(instrument_id="in.x", fast=2, slow=3,
                                         quantity=2, aggressive_limit_bps=Decimal("100"))
        prices = [10, 9, 8, 7, 8, 9, 10]
        intent = None
        for i, mid in enumerate(prices):
            e = event(i, bid=Decimal(mid) - Decimal("0.05"), ask=Decimal(mid) + Decimal("0.05"))
            from quantlab.backtest import StrategyContext
            maybe = strategy.on_event(e, StrategyContext(0, False, Decimal("10000")))
            if maybe is not None:
                intent = maybe
        self.assertIsNotNone(intent)
        self.assertEqual(intent.side, "buy")
        self.assertEqual(intent.quantity, 2)
        down_prices = [9, 8, 7]
        exit_intent = None
        for i, mid in enumerate(down_prices, start=10):
            e = event(i, bid=Decimal(mid) - Decimal("0.05"), ask=Decimal(mid) + Decimal("0.05"))
            maybe = strategy.on_event(e, StrategyContext(2, False, Decimal("9000")))
            if maybe is not None:
                exit_intent = maybe
        self.assertIsNotNone(exit_intent)
        self.assertEqual(exit_intent.side, "sell")
        self.assertEqual(exit_intent.quantity, 2)


class AlphaSearchTests(unittest.TestCase):
    def test_search_is_deterministic_and_uses_untouched_holdout(self):
        bars = []
        price = 100.0
        for i in range(360):
            # Deterministic regime changes; not a return claim.
            drift = (0.0008 if (i // 60) % 2 == 0 else -0.0006)
            noise = (((i * 37) % 11) - 5) * 0.00005
            price *= 1 + drift + noise
            bars.append(Bar(1_800_000_000_000_000_000 + i * 60_000_000_000, price))
        args = dict(costs=AlphaCosts(), candidates=(AlphaConfig("momentum", 5, 0),
                    AlphaConfig("momentum", 10, 5), AlphaConfig("ema_cross", 10, fast=3, slow=10)),
                    min_train=100, min_validation=30, min_holdout=30)
        a, b = search_alpha(bars, **args), search_alpha(bars, **args)
        self.assertEqual(a, b)
        self.assertEqual(a["candidate_family_size"], 3)
        self.assertIn("untouched_holdout", a["splits"])
        self.assertEqual(a["splits"]["embargoed_return_end_bar_indices"], [216, 288])
        self.assertIn("liquidated", a["splits"]["segment_portfolio_policy"])
        self.assertIn(a["status"], {"positive_holdout_screen_passed_not_alpha_certified", "rejected_or_unproven"})
        self.assertEqual(a["periods_per_year_for_sharpe_annualisation"], 252.0)
        self.assertGreater(a["costs"]["one_way_total_bps"], 0)

    def test_rejects_frequency_ambiguous_sharpe_annualisation_value(self):
        bars = [Bar(i + 1, 100.0 + i * 0.1) for i in range(360)]
        with self.assertRaisesRegex(ValueError, "periods_per_year"):
            search_alpha(bars, periods_per_year=float("nan"), min_train=100,
                         min_validation=30, min_holdout=30)

    def test_requires_chronologically_ordered_bars_and_enough_data(self):
        bars = [Bar(i + 1, 100.0 + i) for i in range(200)]
        with self.assertRaises(ValueError): search_alpha(list(reversed(bars)), min_train=10, min_validation=10, min_holdout=10)
        with self.assertRaises(ValueError): search_alpha(bars[:50], min_train=30, min_validation=20, min_holdout=20)


if __name__ == "__main__": unittest.main()
